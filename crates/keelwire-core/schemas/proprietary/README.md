# Proprietary payload schemas

Each `*.json` file in this tree describes the payload of one proprietary
message (or an array of them). Files are embedded into the server binary at
build time and additional directories can be loaded at runtime with
`--schemas <dir>`.

```json
{
  "id": "maretron.clmd16.breaker_config",
  "description": "CLMD16 breaker configuration (Read/Write Switch Config)",
  "manufacturer": 137,
  "industry": 4,
  "pgn": 126720,
  "product_code": 12337,
  "model": "CLMD16",
  "opcodes": [49],
  "read":  { "command": 49, "prefix": "" },
  "write": { "command": 33, "prefix": "" },
  "verified": false,
  "source": "where the layout came from (static analysis, capture file, …)",
  "fields": [
    { "name": "channel", "kind": "uint", "bits": 8 },
    { "name": "current_rating", "kind": "uint", "bits": 16, "scale": 0.1, "unit": "A", "min": 0, "max": 25 },
    { "name": "type", "kind": "enum", "bits": 4,
      "values": [ {"value": 0, "name": "DC"}, {"value": 1, "name": "PWM"} ] },
    { "name": "label", "kind": "string_lau" },
    { "name": "channels", "kind": "group", "count": "channel_count", "fields": [ … ] }
  ]
}
```

`opcodes` lists the reply command byte(s) the layout decodes. `read` / `write`
give the Maretron command byte (and fixed leading data bytes, hex) used to ask
for or send the payload; requests are wrapped in a 126208 command as described
in docs/SPEC.md Appendix A.

Field kinds: `uint`, `int`, `bool`, `enum`, `string_fix` (bits = 8 × bytes),
`string_lau`, `bytes`, `reserved`, `group` (`count` = fixed number or the
name of an earlier field).

**Safety:** schemas with `"verified": false` are decode-only. The server
refuses to write configuration with them unless the user enables
experimental writes for that device. Set `verified` to `true` only after a
read → write → read round trip on real hardware produced identical bytes.

The `maretron/` directory is filled by the protocol-extraction work
(see docs/SPEC.md Appendix A).
