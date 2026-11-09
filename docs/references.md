# References

The outside sources the docs cite: leads and comparison material, never authority over what a unit actually shows. Anything from a peer project is labelled **External** ([evidence levels](evidence.md)) until observed on our own unit.

## Datasheets and filings

| Source | Used for |
| --- | --- |
| [GD25Q64E datasheet](https://download.gigadevice.com/Datasheet/DS-00484-GD25Q64E-Rev1.6.pdf) | PLAF103 external flash ([device](devices/plaf103/README.md)) |
| [FCC 2A3DEPLAF103S internal photos](https://fccid.io/2A3DEPLAF103S/Internal-Photos/Internal-Photos-6465982.pdf) | Board comparison; shows `PLAF103 MB V1.0` |
| [FCC grantee 2A3DE](https://fccid.io/2A3DE) | Every PETLIBRO filing; chip families in [supported devices](devices/README.md) |
| [FCC 2A3DE-PLAF103S1 internal photos](https://fccid.io/2A3DE-PLAF103S1/Internal-Photos/Internal-Photos-7891572) | The 2024 PLAF103 build: RTL8721DM and GD25Q64 on a shielded module, board `PLAF103 MB V1.1` |
| [FCC 2A3DE-PLWF106 internal photos](https://fccid.io/2A3DE-PLWF106/Internal-Photos/Internal-photos-8691584) | PLWF106 board comparison; shows `PLWF106/116 MB Board V0.5` |
| TC118AH datasheet (S&CIC2102 v1.1) | PLAF103 motor driver ([motor driver](devices/plaf103/motor-driver.md)); kept in ignored `private/references/` |
| Realtek AmebaD SDK | Pinned under [`src/firmware/vendor/`](../src/firmware/vendor/README.md) |

## Peer projects

Other people's work on PETLIBRO devices. Different models, different units, and different evidence standards: check a claim on your own unit before relying on it.

| Project | Model | What it offers |
| --- | --- | --- |
| [earlynerd/Petlibro-One-Custom-Firmware](https://github.com/earlynerd/Petlibro-One-Custom-Firmware) | AF301 (mainboard PLNF301), RTL8721DM | A bench-mapped GPIO table for the QFN68 package, RFID Modbus protocol, custom AmebaD firmware |
| [GoliathLabs/petlibro-plwf116-esphome](https://github.com/GoliathLabs/petlibro-plwf116-esphome) | PLWF116, RTL8721DM | ESPHome on a LibreTiny fork, keeping the vendor bootloader and both OTA slots |
| [taylorfinnell/petlibro-esphome](https://github.com/taylorfinnell/petlibro-esphome) | PLAF108, PLWF105 (ESP32-C3) | ESPHome replacement firmware and backup steps; its issues hold teardown reports for other models |
| [aquarat/petlibro-polar-esphome](https://github.com/aquarat/petlibro-polar-esphome) | PLAF109 (ESP32-C3) | ESPHome replacement firmware and reverse-engineering notes |
| [pineconedata/esphome-petlibro-plaf108](https://github.com/pineconedata/esphome-petlibro-plaf108) | PLAF108 (ESP32-C3) | ESPHome replacement firmware with a flashing guide |
| [majabojarska/petlibro-plaf101-jailbreak](https://github.com/majabojarska/petlibro-plaf101-jailbreak) | PLAF101 (ESP8266, Tuya) | A replacement in progress |
| [Dr-Crow/plaf203](https://github.com/Dr-Crow/plaf203) | PLAF203 (Linux, camera) | Root access, boot flow and filesystem notes |
| [jjjonesjr33/petlibro](https://github.com/jjjonesjr33/petlibro) | Cloud API, many models | Home Assistant integration over the vendor cloud |
| [smcneece/petlibro-local](https://github.com/smcneece/petlibro-local) | Many models, stock firmware | Local MQTT control; its device notes map retail models to the codes devices report |
| [icex2/plaf203](https://github.com/icex2/plaf203) | PLAF203S (Linux, camera) | Local control of the MQTT camera feeder |

## Documentation models

Projects whose layout these docs follow: [Asahi Linux docs](https://github.com/AsahiLinux/docs) (per-block hardware pages, feature-support tables), [LibreTiny](https://github.com/libretiny-eu/libretiny) (per-chip and per-board pages, dumping and partition layouts), and [Valetudo](https://github.com/Hypfer/Valetudo) (supported-device lists, rooting and recovery pages).
