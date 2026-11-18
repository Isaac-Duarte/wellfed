# PLWF116 stock communications

Stock 0.3.62's setup, Wi-Fi retry and MQTT paths, for developers reviewing a local replacement or stock-server port. These are recovered code contracts. No app session, radio exchange or cloud connection was observed.

## Setup transports

Both setup transports feed parser `0x0e018db8`:

| Transport | Entry | Recovered behavior |
| --- | --- | --- |
| SoftAP UDP | `0x0e005248` | IPv4 datagram socket, port **17484** (`0x444c`), receives at most 255 bytes per datagram |
| Bluetooth LE | `0x0e026fd0` | Simple-service write callback forwards the supplied pointer and length to the same parser |

SoftAP initializer `0x0e005c44` selects AP mode, address **192.168.1.1**, netmask **255.255.255.0**, channel **11**, and SSID `PL-WF116-` followed by two MAC-derived hexadecimal bytes. It passes security type 0 with no password to the SDK's AP-start call. This is an open AP configuration in code, not a recorded beacon.

Wi-Fi manager `0x0e006a54` starts station mode when both stored Wi-Fi and device-configuration flags are set. Otherwise it starts the setup AP and UDP worker. Bluetooth and Wi-Fi setup events are coordinated by worker `0x0e007010`, rather than letting the parser join Wi-Fi directly.

The setup worker stops the idle-sleep timer while processing configuration. On a Wi-Fi setup attempt it makes at most five joins. Three consecutive SDK errors classified as no-network, codes 3 or 9, end the retry sequence early. Replies carry the connection result, including the recorded SDK error where available.

### Shared DL frame

The shared parser handles a binary frame beginning with ASCII `DL` and ending in `0xff`. Its recovered fields are:

| Byte offset | Meaning |
| --- | --- |
| 0–1 | ASCII `DL` |
| 2 | Operation number |
| 3 | Format / checksum selector; value 1 enables the CRC check |
| 4 | Request sequence byte, echoed in the response |
| 5 | Declared payload length |
| 6 onward | Operation-specific, byte-length-prefixed fields |
| After payload | Two CRC bytes for selector 1, then `0xff`; other selectors omit the CRC |

CRC helper `0x0e010d10` starts its two accumulators at `0xff`, uses two 256-byte lookup tables, and covers the sequence, length and payload. The tables implement the Modbus CRC-16 recurrence. For the conventional `123456789` check input the emitted bytes are `37 4b`; that is a host recomputation of the tables, not a captured frame.

| Operation | Payload meaning / destination |
| --- | --- |
| 1 | Wi-Fi SSID and password, staged through `0x0e00646c` |
| 2 | Broker address and decimal user ID, staged through `0x0e0065b4` |
| 3 | Wi-Fi credentials followed by broker address and user ID |
| 4 | Three identity strings, staged through `0x0e0066c8` |
| 9 | Additional device-information handler `0x0e011e98` |

The parser bounds SSID and password lengths to 32 and 64 bytes in operations 1 and 3. It checks declared payload arithmetic and checks the CRC only when selector 1 is present. Other operations do not show equivalent per-field bounds in this parser. Several field reads precede a complete received-length check. This is not a general safe decoder for untrusted or truncated frames.

No application encryption, keyed authenticator or pairing proof appears in this shared parser. A CRC detects accidental changes and does not authenticate the sender. The BLE attribute and pairing policy below likewise do not require an authenticated setup write.

Response builder `0x0e018d78` emits `DL`, operation, selector, sequence, a one-byte payload and the result. It produces eight bytes without CRC and ten with CRC. Once Wi-Fi and device information are already configured and the saved Wi-Fi record is non-erased, the parser diverts operations 1–3 to a rejection/result event. That guard is state-based, not a cryptographic authorization mechanism.

### Saving settings and identity

Configuration worker `0x0e007010` separates staging, connection attempts and persistence. It writes the 96-byte Wi-Fi record at `0x3e9000` and the device-information block at `0x3e8000` along successful setup paths. Broker configuration and cloud connection participate in finishing setup; receiving a frame alone does not prove credentials were saved.

Identity provisioning writes a 150-byte record at `0x3ed000`, then reads it back for comparison. This is a factory-capable operation, distinct from joining Wi-Fi. Preserve the original record and do not treat the setup protocol as a reason to recreate unit identity.

Stock logs Wi-Fi credentials, broker configuration and identity fields on these paths. Captures and complete dumps belong in private evidence.

### BLE battery service is not the power gauge

The battery-service read branch in `0x0e026fd0` supplies the fixed byte **`0x5a` (90)**. It does not read the ADC-based battery percentage. A BLE battery-service reading therefore cannot validate the [power manager's gauge](../power.md); the real percentage reaches the cloud through attributes.

### BLE attributes and pairing policy

Service registration `0x0e02ea88` supplies the 336-byte database at `0x0e0befbc`. Decoding its twelve 28-byte entries with the SDK's `T_ATTRIB_APPL` layout gives:

| Service / characteristic | UUID | Properties and permissions |
| --- | --- | --- |
| Primary setup service | `0xffee` | Readable service declaration |
| V1 read | `0xb001` | Read; ordinary read permission |
| V2 setup write | `0xffe1` | Write and write without response; ordinary write permission `0x10` |
| V3 reply notification | `0xffe2` | Notify; CCCD has ordinary read/write permission `0x11` |
| V4 indication | `0xb004` | Indicate; CCCD has ordinary read/write permission `0x11` |

The UUIDs are vendor choices in this image; the SDK example's different primary UUID does not describe the shipped table. The write attribute does not set the SDK's authenticated, encrypted or authorized-write permission bits.

BLE initializer `0x0e004f34` sets device name `PLWF116`, pairable mode 1, bonding flag 1 and I/O capability 3, meaning no input or output. It disables out-of-band pairing, the fixed passkey and the peripheral's automatic security request. The bonding flag does not make ordinary GATT writes require a bonded connection, and no MITM flag is selected. This describes configured policy; no pairing or unencrypted write was exercised on a unit.

The application forwards each GATT write to the DL parser. It does not assemble a fragmented DL frame there. ATT MTU, long-write behavior and the app's framing need a capture before building an interoperable client.

## Wi-Fi retry and sleep

The Wi-Fi manager serializes normal, periodic and radar-triggered reconnect work through one queue. A radar event does not launch another parallel join task. State at `0x10020974` tracks retry kind, counters, periodic mode and pending/active requests.

| Path | Code anchor | Recovered policy |
| --- | --- | --- |
| Normal reconnect | `0x0e006034` | Wait 8,000 ms before each join; after a failed attempt, normally wait another 1,000 ms |
| Periodic battery reconnect | `0x0e006034`, `0x0e006a54` | One failed join per wake, then select **360,000 ms / six minutes** for the next retry |
| Radar wake reconnect | `0x0e005b0c`, `0x0e006a54` | Only with complete stored setup and periodic mode active; at most **two failed joins** per request |
| Unconfigured idle | `0x0e006770`, `0x0e005a08` | One-shot **1,200,000 ms / 20-minute** timer; sleep action runs only on battery power |

Normal failures increment a counter. After 18 failures, radar watering mode increments a persisted failure-cycle byte at `0x3ec000`. Above four recorded cycles, a battery-powered unit enters periodic mode. Other paths report an error and request reset rather than establishing the same sleep transition. These are counts and scheduled delays, not a measured elapsed-time guarantee: association and DHCP add time.

Periodic failures keep the six-minute interval and release the Wi-Fi-related wake locks. With external power, these branches skip their battery sleep behavior. A successful join clears the normal and radar counters, clears periodic state, writes zero to the failure-cycle byte, resumes application control work and releases the reconnect locks.

Radar helper `0x0e005b0c` checks that setup is complete, periodic mode is active and the Wi-Fi manager exists. Under a critical section it refuses duplicate pending or active requests, records radar retry type 2, clears its counter, takes lock 19 and signals the manager. The manager checks those conditions again before running; a stale request is canceled and its lock released.

After two failed radar attempts, battery operation waits for the sensing-duration setting plus a small margin before releasing the reconnect and OS locks. Pump-on time and Wi-Fi availability therefore need not match: radar can request local circulation even when reconnection fails. The [power reference](../power.md) follows the suspend and resume callbacks and wake sources.

The idle callback explicitly skips DC and battery-plus-DC operation. On battery it requests station mode, releases locks 1 and 26 and enters the ordinary sleep preparation path. The separate setup-event hook `0x0e006424` stops this timer while configuring. The printed “20 minutes” applies to the one-shot idle timer, not every battery sleep or retry.

## MQTT transport and topics

MQTT worker `0x0e01349c` initializes its network object through `0x0e0011bc`. That initializer sets the TLS-enable byte at object offset `0x18` to zero and clears its certificate pointers. Connector `0x0e0011f0` has a TLS branch, but the reviewed worker's initialization selects ordinary TCP. The presence of mbedTLS code and certificate parsing does not make this MQTT path encrypted.

Topic builder `0x0e012a00` uses the model string `PLWF116`:

| Topic pattern | Purpose |
| --- | --- |
| `dl/PLWF116/<device>/device/<channel>/post` | Device publishes |
| `dl/PLWF116/<device>/device/<channel>/sub` | Device-targeted messages |
| `dl/PLWF116/device/broadcast/sub` | Model broadcast |
| `dl/PLWF116/<device>/device/+/sub` | All device channels |

Channels are `heart`, `ota`, `ntp`, `config`, `event`, `service` and `system`. Subscription helper `0x0e0132f0` subscribes to the all-channel and broadcast topics using QoS argument 1. Keepalive configuration is 120 seconds in the MQTT worker. This is the stock cloud protocol, not Wellfed's device API.

The CONNECT object selects MQTT version **3**, meaning MQTT 3.1, keeps the connection alive at **120 seconds**, and sets `cleanSession` to zero. Its strings come directly from the 150-byte identity record:

| Record offset | Length of reserved slot | CONNECT field |
| --- | --- | --- |
| 0 | 50 bytes | Client ID / device serial |
| 50 | 50 bytes | Username / product key |
| 100 | 50 bytes | Password / product secret |

The worker passes pointers to those slots directly to CONNECT serialization; no password derivation occurs between them. The slots' sizes do not prove arbitrary 50-byte input is safely terminated. The selected unit's values remain unknown without its private record.

The worker rotates through stored backup broker addresses during reconnect failures. Initialization `0x0e006770` reads their 521-byte list from `0x7d3000`; an accepted count is one through five. An OTA payload does not contain the selected unit's actual broker credentials or current binding state.

### Incoming commands

Receive callback `0x0e01299c` passes messages to a queue; dispatcher `0x0e017bdc` parses JSON and selects these recovered handlers. The following list records routing, not a claim that every command is safe or every malformed request is rejected.

| Message / identifier | Handler | Role |
| --- | --- | --- |
| `NTP`, `NTP_SYNC` | `0x0e015dd8`, `0x0e016058` | Time synchronization |
| `OTA_UPGRADE` | `0x0e016ea8` | [OTA request](ota-requirements.md) |
| `GET_CONFIG` | Wakes configuration reporting | Ask for configuration |
| `BINDING`, `UNBIND` | `0x0e016118`, `0x0e0161c0` | Binding state |
| `WEIGHT_CLR_SERVICE`, `WEIGHT_CAL_SERVICE` | `0x0e017a18`, `0x0e017b60` | Scale calibration control |
| `WIFI_RECONNECT_SERVICE`, `WIFI_CHANGE_SERVICE` | `0x0e016344`, `0x0e0163dc` | Wi-Fi management |
| `ATTR_SET_SERVICE`, `ATTR_GET_SERVICE` | `0x0e016558`, `0x0e016c34` | Settings writes and reads |
| `SERVER_CONFIG_PUSH` | `0x0e016ccc` | Server-configuration acknowledgement path |
| `RESTORE` | `0x0e0170f4` | Reset attributes |
| `ATTR_PUSH_EVENT`, `DEVICE_START_EVENT`, `ERROR_EVENT` | `0x0e0171bc`, `0x0e017230`, `0x0e017278` | Report responses |
| `DEVICE_FUNCTION_TEST_SERVICE` | `0x0e017388` | Factory controls, including pump-related tests |
| `TRIGGER_DEVICE_LOG_REPORT_SERVICE`, `DEVICE_LOG_SERVICE` | `0x0e017704`, `0x0e017784` | Logging controls |
| `DEVICE_FILE_UPLOAD_EVENT` | `0x0e028dac`, `0x0e0289c0` | Upload URL response, checked against pending message ID |
| `DEVICE_CONFIG_SYNC` | `0x0e0177f4` | Device-configuration synchronization |
| `DEVICE_PRINT_LOG_SWITCH_SERVICE` | `0x0e017980` | Console logging control |
| `DEVICE_REBOOT`, `SET_CHANNEL_PLAN` | `0x0e0162a4`, `0x0e015cc0` | Reboot and Wi-Fi channel policy |
| `DEVICE_PROPERTIES_SERVICE` | `0x0e017020` | Property handler present; dispatcher lookup inconsistency below |

The dispatcher performs two lookups of the same `cmd` key at `0x0e017c72` and `0x0e017c78`. Both load pool word `0x0e017ea0`, which points to the exact NUL-terminated string `cmd` at `0x0e0b08b0`. In its `SERVICE` branch it then compares the second result with `DEVICE_PROPERTIES_SERVICE`, despite that value already being `SERVICE`. For ordinary JSON these conditions cannot both hold. The presence of the property handler therefore does not establish reachable `SERVICE` / `identifier` dispatch; this is a recovered inconsistency, with no observed app consequence.

Read [stock control](README.md) before interpreting factory or calibration commands. MQTT dispatch and a `msgId` are not proof of application-level command authentication. Broker access control was not available from this OTA file.

### Telemetry and power meaning

State reporter `0x0e015268` emits `ATTR_PUSH_EVENT`, including `useWaterType`, interval/duration for interval mode, and `lowWater`. When its power-change flag is set, it also includes:

- `powerType`, from the power attribute;
- `electricQuantity`, the computed percentage;
- `batteryState`, mapping stored state 0 to `CHARGING`, 1 to `CHARGED` and 2 to `USING`.

These values report the firmware's interpretation of GPIOs and voltage. They do not measure charge current or remaining watt-hours.

Event encoder `0x0e013e38` creates `EVENT` / `DEVICE_DATA_EVENT` with `weight`, `radarState` and `waterPumpState`. Water-history encoder `0x0e015428` creates `WATER_CAP_EVENT`, dividing records into groups of at most **45**, with `batchIndex`, `totalBatches` and a `content` array. Each record carries `weightPercent`, `weight`, `radarDetected` and a timestamp. A load change plus radar is telemetry, not a proven measurement of water consumed.

OTA reporting has its own `OTA_INFORM` and `OTA_PROGRESS` messages. Their encoders are `0x0e013f00` and `0x0e01407c`; a successful report is separate from an application's first boot.

## Diagnostic-file upload

Uploader `0x0e028298` is a separate path from OTA. It first requires network state 2, an MQTT binding, and a nonempty local file. It requests an upload URL through MQTT, then waits up to **30 seconds** for the response.

The receive dispatcher checks a supplied message ID against the pending ID with `0x0e028dac`; a mismatch is dropped. The helper accepts a missing or empty response ID, so this is not a strict requirement that every response prove its association. URL setter `0x0e0289c0` accepts fewer than 1,024 bytes.

Unlike OTA, this uploader selects a secure HTTP connection for `https://` and a plain connection for `http://`. It accepts a host of at most 127 bytes and a resource of at most 1,023 bytes. It sends **PUT**, a content type and length, and `Connection: close`, then copies the file in blocks of at most **1,024 bytes**. It rejects short writes and treats an HTTP **2xx** response as success. It attempts 15-second send and receive socket timeouts.

The uploader closes the file and connection and reports its result through the upload event/callback path. The `TLS connect ok` message belongs here; it does not describe the OTA downloader.

### HTTPS does not authenticate the upload server

At `0x0e0284de..0x0e0284e4`, the caller explicitly passes zero for the client certificate, client key and trusted CA arguments to connection constructor `0x0e05ad18`. That constructor forwards them to TLS initializer `0x0e02015c`.

The TLS initializer selects **`MBEDTLS_SSL_VERIFY_NONE`**, through `0x0e032bf0` at `0x0e020202`. It switches to required verification only when a non-null CA argument reaches its branch at `0x0e020268`. The uploader supplies no CA, so that branch is skipped. Its HTTPS connection encrypts traffic but does not require a trusted server certificate.

Connection helper `0x0e05ae74` forwards the requested host to handshake helper `0x0e020338`. This calls hostname setter `0x0e032c24` before the TLS handshake. Setting that name does not enable certificate verification; the uploader retains verification mode zero. No separate certificate-result or hostname-match rejection appears in this constructor-to-handshake chain.

The call immediately after constructing the connection, `0x0e05b104`, only sets the connection's `ignore_content_len` field to 1. It does not install certificates or enable verification. These are static findings about the shipped calls, not an exercised interception test.

## Limits for a stock-server port

The topic and message names explain why the feeder and fountain share infrastructure. They do not establish that the PLAF103 server can serve the fountain unchanged. A port needs typed fountain messages, real login and binding fixtures, setup replies, clock behavior, history batching, reconnect/sleep tests and a reviewed OTA profile.

The remaining protocol questions include actual app framing and BLE transfer sizes, broker-side login and binding policy, all JSON type and length checks, the inconsistent property-service dispatch, and behavior across diagnostic-upload interruptions. These are separate from the recovered plain HTTP OTA path. No network service was started for this analysis.

## Evidence

- **Recovered, 2026-11-16:** static review of stock 0.3.62 setup transport callers, shared DL parser, CRC tables, setup reply and configuration worker; AP/socket constants and field-length arguments checked in instruction listings.
- **Recovered, 2026-11-16:** Wi-Fi manager, reconnect worker, radar queue, persisted failure cycle and idle timer. Function-entry corrections preserved initial argument setup before reviewing branches.
- **Recovered, 2026-11-16:** MQTT network initialization, topic builder, subscriptions, receive dispatch, state/event/history encoders, the GATT attribute database and pairing initialization, and the fixed BLE battery-service response. Attribute layouts and numeric permissions were cross-checked with the local SDK headers. The diagnostic-upload constructor, TLS initializer, auth-mode setter, connection and hostname/handshake helpers establish the absence of required certificate verification; call arguments were checked in listings. No physical, cloud or app exchange is claimed.
