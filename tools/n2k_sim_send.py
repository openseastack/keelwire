#!/usr/bin/env python3
"""Send scripted NMEA 2000 traffic for testing Keelwire (standard library only).

Outputs:
  --udp HOST:PORT     Yacht Devices RAW lines over UDP   (keelwire: ydraw:udp://0.0.0.0:PORT)
  --tcp-server PORT   Yacht Devices RAW lines to TCP clients (keelwire: ydraw:tcp://127.0.0.1:PORT)
  --candump FILE      candump -L log for replay           (keelwire: replay:FILE)
  --canboat FILE      CANboat plain CSV for replay or Signal K input

Scenarios: basic (GPS, heading, wind, battery, tanks, switch bank),
conflict (two tank senders on the same instance), fastpacket (long messages).

Examples:
  python3 n2k_sim_send.py --udp 127.0.0.1:1456 --scenario conflict
  python3 n2k_sim_send.py --candump test.log --seconds 30
"""

import argparse
import math
import random
import socket
import struct
import sys
import threading
import time
from datetime import datetime, timezone


def can_id(prio, pgn, src, dst=255):
    pf = (pgn >> 8) & 0xFF
    dp = (pgn >> 16) & 0x3
    ps = dst if pf < 0xF0 else pgn & 0xFF
    return (prio & 7) << 26 | dp << 24 | pf << 16 | ps << 8 | src


FAST = {126208, 126464, 126996, 126998, 129029, 127489, 129540}
_seq = {}


def frames_for(pgn, data):
    """Split a payload into CAN frames (fast-packet when needed)."""
    if pgn not in FAST and len(data) <= 8:
        return [data + b"\xff" * (8 - len(data)) if len(data) < 8 else data]
    s = _seq.get(pgn, 0)
    _seq[pgn] = (s + 1) & 7
    out = [bytes([s << 5, len(data)]) + data[:6]]
    pos, idx = 6, 1
    while pos < len(data):
        chunk = data[pos:pos + 7]
        out.append(bytes([(s << 5) | idx]) + chunk + b"\xff" * (7 - len(chunk)))
        pos += 7
        idx += 1
    out[0] = out[0] + b"\xff" * (8 - len(out[0]))
    return out


def name_bytes(unique, mfr, cls, func, inst=0):
    v = unique | mfr << 21 | (inst & 7) << 32 | (inst >> 3) << 35 | func << 40 | cls << 49 | 4 << 60 | 1 << 63
    return struct.pack("<Q", v)


def sfix(s, n=32):
    b = s.encode()[:n]
    return b + b"\xff" * (n - len(b))


def product(model, code, sw, serial):
    return struct.pack("<HH", 2100, code) + sfix(model) + sfix(sw) + sfix("Rev 1") + sfix(serial) + bytes([2, 1])


class Scenario:
    def __init__(self, kind):
        self.kind = kind
        self.t = 0.0
        self.devices = [
            (30, name_bytes(9001, 2046, 60, 145), product("PyGPS", 1, "1.0", "PY0001")),
            (31, name_bytes(9002, 2046, 60, 140), product("PyCompass", 2, "1.0", "PY0002")),
            (32, name_bytes(9003, 2046, 75, 150, 0), product("PyTank", 3, "1.0", "PY0003")),
            (33, name_bytes(9004, 2046, 75, 150, 1), product("PyTank", 3, "1.0", "PY0004")),
            (34, name_bytes(9005, 2046, 35, 170), product("PyBattery", 4, "1.0", "PY0005")),
            (35, name_bytes(9006, 2046, 30, 140), product("PySwitch", 5, "1.0", "PY0006")),
        ]

    def startup(self):
        for addr, name, prod in self.devices:
            yield 6, 60928, addr, 255, name
            yield 6, 126996, addr, 255, prod

    def tick(self, dt):
        """Yield (prio, pgn, src, dst, payload) for one 100 ms step."""
        self.t += dt
        t = self.t
        step = round(t * 10)
        lat, lon = 26.0123 + 0.001 * math.sin(t / 60), -80.1234 + 0.001 * math.cos(t / 60)
        yield 2, 129025, 30, 255, struct.pack("<ii", int(lat * 1e7), int(lon * 1e7))
        yield 2, 127250, 31, 255, struct.pack("<BHhhB", step & 0xFF, int((1.2 + 0.3 * math.sin(t / 20)) * 10000) % 62832, 0x7FFF, -1200, 0xFD)
        if step % 2 == 0:
            yield 2, 130306, 31, 255, struct.pack("<BHHB", step & 0xFF, int((7 + 2 * random.random()) * 100), int(0.8 * 10000), 0xFA) + b"\xff\xff"
        if step % 10 == 0:
            yield 6, 127508, 34, 255, struct.pack("<BHhHB", 0, int(1285 + 10 * math.sin(t / 30)), -52, int((24 + 273.15) * 100), step & 0xFF)
            inst2 = 0 if self.kind == "conflict" else 1
            for addr, inst, lvl in ((32, 0, 62.0), (33, inst2, 48.0)):
                yield 6, 127505, addr, 255, bytes([(inst & 0xF)]) + struct.pack("<hI", int((lvl + random.random()) / 0.004), 4000) + b"\xff"
        if step % 20 == 0:
            states = [1, 0, 1, 1, 0, 0, 1, 0] + [3] * 20
            b = 0
            for i, s in enumerate(states):
                b |= s << (2 * i)
            yield 3, 127501, 35, 255, bytes([0]) + b.to_bytes(7, "little")
        if self.kind == "fastpacket" and step % 10 == 5:
            # 129029 GNSS position (43 bytes)
            d = struct.pack("<BHIqqqBBBhhiB", step & 0xFF, 20384, int((t % 86400) * 10000), int(lat * 1e16), int(lon * 1e16), 150000, 0x13, 0xFC, 9, 90, 150, -2500, 0)
            yield 3, 129029, 30, 255, d


def run(args):
    sc = Scenario(args.scenario)
    sinks = []
    if args.udp:
        host, port = args.udp.rsplit(":", 1)
        u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        u.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
        sinks.append(lambda line, *_: u.sendto(line.encode(), (host, int(port))))
    clients = []
    if args.tcp_server:
        srv = socket.socket()
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind(("0.0.0.0", args.tcp_server))
        srv.listen()

        def accept():
            while True:
                c, a = srv.accept()
                print(f"client {a}", file=sys.stderr)
                clients.append(c)

        threading.Thread(target=accept, daemon=True).start()

        def tcp_send(line, *_):
            for c in list(clients):
                try:
                    c.sendall(line.encode())
                except OSError:
                    clients.remove(c)

        sinks.append(tcp_send)
    cd = open(args.candump, "w") if args.candump else None
    cb = open(args.canboat, "w") if args.canboat else None
    realtime = not (cd or cb) or bool(args.udp or args.tcp_server)
    t0 = time.time()

    def emit(msgs, now):
        for prio, pgn, src, dst, data in msgs:
            ident = can_id(prio, pgn, src, dst)
            if cb:
                ts = datetime.fromtimestamp(now, timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"
                cb.write(f"{ts},{prio},{pgn},{src},{dst},{len(data)}," + ",".join(f"{x:02x}" for x in data) + "\n")
            for fr in frames_for(pgn, data):
                tod = datetime.fromtimestamp(now).strftime("%H:%M:%S.%f")[:-3]
                line = f"{tod} R {ident:08X} " + " ".join(f"{x:02X}" for x in fr) + "\r\n"
                for s in sinks:
                    s(line)
                if cd:
                    cd.write(f"({now:.6f}) can0 {ident:08X}#{fr.hex().upper()}\n")

    emit(sc.startup(), t0)
    n = int(args.seconds * 10) if args.seconds else None
    i = 0
    while n is None or i < n:
        now = t0 + i * 0.1 if not realtime else time.time()
        emit(list(sc.tick(0.1)), now)
        if (i % 600) == 0:
            emit(sc.startup(), now)  # periodic re-claims so late listeners see devices
        i += 1
        if realtime:
            time.sleep(max(0, t0 + i * 0.1 - time.time()))
    for f in (cd, cb):
        if f:
            f.close()


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--udp")
    p.add_argument("--tcp-server", type=int)
    p.add_argument("--candump")
    p.add_argument("--canboat")
    p.add_argument("--scenario", choices=["basic", "conflict", "fastpacket"], default="basic")
    p.add_argument("--seconds", type=float, help="stop after N seconds (required for file output without network sinks)")
    a = p.parse_args()
    if not (a.udp or a.tcp_server or a.candump or a.canboat):
        p.error("choose at least one output")
    if (a.candump or a.canboat) and not (a.udp or a.tcp_server) and not a.seconds:
        a.seconds = 60
    try:
        run(a)
    except KeyboardInterrupt:
        pass
