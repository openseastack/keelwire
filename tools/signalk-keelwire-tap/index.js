// Signal K server plugin: exposes ws://<server>/plugins/signalk-keelwire-tap/stream
// carrying every NMEA 2000 message as {prio,pgn,src,dst,data:"hex"}.
// Messages sent by Keelwire in the same format are put on the bus via
// Signal K's 'nmea2000out' event when "allowTransmit" is enabled.
const { WebSocketServer } = require("ws");
const { toPgn } = require("@canboat/canboatjs");

module.exports = function (app) {
  const plugin = { id: "signalk-keelwire-tap", name: "Keelwire tap" };
  let wss = null;
  let onJson = null;

  plugin.schema = {
    type: "object",
    properties: {
      allowTransmit: { type: "boolean", title: "Allow Keelwire to transmit on the bus", default: false },
    },
  };

  plugin.start = function (options) {
    wss = new WebSocketServer({ noServer: true });
    const path = "/plugins/signalk-keelwire-tap/stream";
    app.server.on("upgrade", (req, socket, head) => {
      if (req.url && req.url.startsWith(path)) wss.handleUpgrade(req, socket, head, (ws) => wss.emit("connection", ws, req));
    });
    const broadcast = (msg) => {
      const s = JSON.stringify(msg);
      for (const c of wss.clients) if (c.readyState === 1) c.send(s);
    };
    // canboatjs emits decoded JSON on its analyzer-output event (the event name
    // below is canboatjs's API); re-encode to bytes.
    onJson = (pgn) => {
      try {
        const buf = toPgn(pgn);
        if (!buf) return;
        broadcast({ prio: pgn.prio, pgn: pgn.pgn, src: pgn.src, dst: pgn.dst, data: Buffer.from(buf).toString("hex") });
      } catch (e) {
        app.debug(`encode failed for ${pgn.pgn}: ${e.message}`);
      }
    };
    app.on("N2KAnalyzerOut", onJson);
    wss.on("connection", (ws) => {
      app.debug("Keelwire connected");
      ws.on("message", (raw) => {
        if (!options.allowTransmit) return;
        try {
          const m = JSON.parse(raw.toString());
          const bytes = Buffer.from(m.data, "hex");
          // Actisense/CANboat plain format, accepted by Signal K's nmea2000out.
          const hex = [...bytes].map((b) => b.toString(16).padStart(2, "0")).join(",");
          app.emit("nmea2000out", `${new Date().toISOString()},${m.prio ?? 6},${m.pgn},0,${m.dst ?? 255},${bytes.length},${hex}`);
        } catch (e) {
          app.debug(`bad message from Keelwire: ${e.message}`);
        }
      });
    });
  };

  plugin.stop = function () {
    if (onJson) app.removeListener("N2KAnalyzerOut", onJson);
    if (wss) wss.close();
    wss = null;
  };

  return plugin;
};
