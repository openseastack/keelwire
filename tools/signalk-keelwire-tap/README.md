# signalk-keelwire-tap

Signal K server plugin that lets Keelwire read (and optionally write)
the NMEA 2000 bus your Signal K server is connected to, without taking the
gateway away from Signal K.

Install into `~/.signalk`: `npm install /path/to/tools/signalk-keelwire-tap`,
restart Signal K, enable "Keelwire tap" under Server → Plugin Config,
then connect Keelwire to:

    signalk:ws://<signalk-host>:3000/plugins/signalk-keelwire-tap/stream

Add `?tx=1` to the URI and enable "Allow Keelwire to transmit" in the
plugin to send. Signal K's own NMEA 2000 output must be configured for its
gateway.

Note: messages are re-encoded from canboatjs' decoded form, so proprietary
or unknown PGNs that canboatjs cannot encode are not forwarded. For complete
raw traffic, use a gateway directly (SocketCAN, IPG100, YD RAW).

To simulate traffic, Signal K's plugins such as `signalk-n2k-virtual-switch`
or the Python sender in `../n2k_sim_send.py` can be used.
