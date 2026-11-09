# Supported devices

Wellfed runs on one model: the PETLIBRO Granary PLAF103, a feeder. Every model is of a kind; Wellfed has two, a feeder and a water fountain. This page lists PETLIBRO's connected models we know of, grouped by chip family, and how a new one is added.

A model counts as supported only once a unit of it has been through our bench. Sharing a chip is not enough: boards, pins, flash layouts and stock firmware differ between models.

| Model | Kind | Product | FCC ID | SoC | Status | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| [PLAF103](plaf103/README.md) | Feeder | Granary 5 L | 2A3DEPLAF103S | [RTL8721DM](../chips/rtl8721dm/README.md) | **Partial** | Our research device. Feeding, updates and the device API work; see [the roadmap](../roadmap.md). Earlier and later builds differ ([how to identify it](plaf103/README.md#how-to-identify-it)). |
| [PLWF106](plwf106/README.md) | Fountain | Dockstream 2 Smart Fountain | 2A3DE-PLWF106 | [RTL8721DM](../chips/rtl8721dm/README.md) | **Experimental** | Stock firmware and board reference recovered. A [boot-test image](plwf106/boot-test.md) builds and passes its audits, but has not run on a fountain. |
| PLWF116 | Fountain | Dockstream 2 Cordless | 2A3DE-PLWF116 | RTL8721DM (External) | Not supported | Its FCC filing names the same main board as the PLWF106. An ESPHome port on LibreTiny runs on it. |
| PLAF301 | Feeder | One RFID | 2A3DE-PLAF301S | RTL8721DM (External) | Not supported | Also called AF301, mainboard PLNF301. FCC photos show the PLAF103's 34-pad module. A peer project maps its pins, a lead for our [pin table](../chips/rtl8721dm/pins.md). |
| PLAF107 | Feeder | Space | 2A3DE-PLAF107 | AmebaD (External) | Not supported | The FCC report names Realtek's AmebaD test tool. The part number is unreadable in the photos. |
| PLAF105 | Feeder | Granary 2 | 2A3DE-PLAF105 | Unknown | Not supported | Launched June 2026. Dual-band Wi-Fi and BLE; the public filing names no chip. |
| PLAF205 | Feeder | Granary 2 Vision and Vision Duo | 2A3DE-PLAFX05 | Unknown | Not supported | Launched June 2026, with an AI camera and a 24 GHz radar. One filing covers PLAF205 and PLAF305. |
| PLAF108 | Feeder | Air | 2A3DE-PLAF108S | ESP32-C3 (External) | Not supported | ESP32-C3-WROOM-02. Several ESPHome replacements exist outside this project ([references](../references.md#peer-projects)). |
| PLAF109 | Feeder | Polar wet food feeder | 2A3DE-PLAF109 | ESP32-C3 (External) | Not supported | ESP32-C3-WROOM-02U with 4 MB of flash. An ESPHome replacement exists. |
| PLWF105 | Fountain | Dockstream Smart Fountain | 2A3DE-PLWF105 | ESP32-C3 (External) | Not supported | ESP32-C3-WROOM-03. An ESPHome replacement exists. |
| PLWF305 | Fountain | Dockstream RFID Smart Fountain | 2A3DE-PLWF305 | Espressif (External) | Not supported | The FCC report names Espressif's test tool. The module is not known. |
| PLAF203 | Feeder | Granary Smart Camera Feeder | 2A3DE-PLAF203U | Embedded Linux (External) | Not supported | A Goke GK7205 camera SoC with an RTL8733BU Wi-Fi chip. Older Tuya units use an Anyka SoC. Root access notes exist outside this project. |
| PLLB001 | Litter box | Luma Smart Litter Box | 2A3DE-PLLB001 | Unknown | Not supported | Dual-band Wi-Fi with classic Bluetooth, which suggests a Linux SoC. No teardown found. |
| PLAF101 | Feeder | Automatic feeder (Tuya) | — | ESP8266 (External) | Not supported | Runs on the Tuya cloud and the PETLIBRO Lite app, not PETLIBRO's own. A replacement is in progress outside this project. |

**Not supported** means we have done no work on that model, and anything listed for it comes from FCC filings or other projects. The FCC ID shown is the latest filing; earlier ones may show other boards.

The best sellers appear to be the Granary (PLAF103 and the camera PLAF203), the One RFID and the Polar, judged by PETLIBRO's store labels and retail review counts. Every model on the PETLIBRO app talks to the same PETLIBRO MQTT broker, whatever its chip. The fountains PLWF005 and PLWF115 have no radio: their FCC filings cover only the inductive link to the pump.

> [!NOTE]
> The model on the box may differ from the one the feeder reports over the network. Peer projects report a One RFID listed as a PLAF103 that reports PLAF301, and fountains labelled PLWF105 that report PLWF106. Trust the FCC ID on the label and the board markings.

## Adding a device

A new model starts as **Not supported**, and moves up once its own unit passes the read-only steps: identification, a receive-only UART capture and matching full backups.

1. Copy [`_template/`](_template/README.md) to `docs/devices/<model>/` and fill in every section, giving each finding its [evidence level](../evidence.md).
2. Add the model's column to the [chip pin table](../chips/rtl8721dm/pins.md).
3. Put its machine-readable layout and profile in `src/firmware/devices/<model>/`, next to [`plaf103`](../../src/firmware/devices/plaf103/). The [device layer](../firmware/device-layer.md) explains how firmware picks them up.
4. Add a row to the table above.

A model of an existing kind shares that kind's crates and adds a board crate and a firmware. A new kind also brings its own drivers, policy, stores, device API module and simulator. [Device kinds and models](../firmware/device-kinds.md) follows a fountain through each layer.

## Evidence

All of it **External**, read on November 9, 2026 and not checked on a unit.

- FCC filings under grantee code 2A3DE (Shenzhen Libro Technology Co., Ltd.), listed on [fccid.io](https://fccid.io/2A3DE). None is a modular approval, so the chip comes from internal photos and test reports. The [PLAF103S1](https://fccid.io/2A3DE-PLAF103S1/Internal-Photos/Internal-Photos-7891572) and [PLAF301S](https://fccid.io/2A3DE-PLAF301S/Internal-Photos/Internal-photos-8536085) photos show a legible RTL8721DM and GigaDevice 25Q64 flash on one 34-pad shielded module. The [PLAF203U](https://fccid.io/2A3DE-PLAF203U/Internal-Photos/Internal-photos-8527729) photos show the Goke SoC and RTL8733BU. The PLAF107, PLAF108, PLAF109, PLWF105 and PLWF305 test reports name the vendor's RF test tool, which identifies the chip family only.
- [Peer projects](../references.md#peer-projects) for the ESP32-C3 modules, the PLWF116 and PLAF301 boards, the camera feeders and the PLAF101.
- Product names and launch dates from PETLIBRO's store and its June 2026 Granary 2 press release.
