# Board embedding report espressif/esp32-s3/esp32-s3-devkitc-1/n16r8-v1.1

- board: espressif/esp32-s3/esp32-s3-devkitc-1/n16r8-v1.1 0.1.0 (hardware revision v1.1), pack digest `46ac681cd7f3f275`
- carrier: sensor-carrier
- placement: U1 (rotation 0°)
- status: **needs_attention**

## 1. What the board fixes (read-only)

The board body, its headers, outline and the keep-outs below are fixed on the carrier; nothing may move, re-route or cover them:

- `REG-BOARD-BODY` (module_body) [[27.3,37.1],[52.7,37.1],[52.7,99.965],[27.3,99.965]]
- `REG-DK-ANT` (antenna_keepout) [[31.05,99.94],[49.05,99.94],[49.05,106.25],[31.05,106.25]]
- `REG-DK-USB-J2` (connector) [[29.33,36.227],[37.27,36.227],[37.27,37.1],[29.33,37.1]]
- `REG-DK-USB-J4` (connector) [[42.625,36.227],[50.025,36.227],[50.025,37.1],[42.625,37.1]]

## 2. Header pins that may be used

24 (brought out by the board, with evidence): J1.10, J1.11, J1.12, J1.15, J1.16, J1.17, J1.18, J1.19, J1.20, J1.4, J1.5, J1.6, J1.7, J1.8, J1.9, J3.16, J3.17, J3.18, J3.4, J3.5, J3.6, J3.7, J3.8, J3.9

## 3. Header pins that may not be used, or are unknown

- `J1.1` 3V3 — reserved: a power pin of the board (guide: Power Supply Options: 5V and G, 3V3 and G, or USB; mutually exclusive): not a signal pin, in no pool
- `J1.2` 3V3 — reserved: a power pin of the board (guide: Power Supply Options: 5V and G, 3V3 and G, or USB; mutually exclusive): not a signal pin, in no pool
- `J1.3` RST — onboard: EN (CHIP_PU): the Reset button and its RC circuit are on the board; the pin is the board's reset line, not a signal pin
- `J1.13` GPIO3 — unknown: GPIO3 is a strapping pin of the module (module pack rule C-ESP-STRAP-IO3); the board documents list it on the header and do not say that it is free for external circuits: nothing is assumed, and the pin is in no pool
- `J1.14` GPIO46 — unknown: GPIO46 is a strapping pin of the module (module pack rule C-ESP-STRAP-IO46); the board documents list it on the header and do not say that it is free for external circuits: nothing is assumed, and the pin is in no pool
- `J1.21` 5V — reserved: a power pin of the board (guide: Power Supply Options: 5V and G, 3V3 and G, or USB; mutually exclusive): not a signal pin, in no pool
- `J1.22` GND — reserved: ground: not a signal pin, in no pool
- `J3.1` GND — reserved: ground: not a signal pin, in no pool
- `J3.2` TX — onboard: U0TXD / U0RXD (UART0): the same nets as the on-board USB-to-UART bridge, which the board uses to flash and talk to the chip (user guide; schematic sheet 2, read from the drawing)
- `J3.3` RX — onboard: U0TXD / U0RXD (UART0): the same nets as the on-board USB-to-UART bridge, which the board uses to flash and talk to the chip (user guide; schematic sheet 2, read from the drawing)
- `J3.10` GPIO38 — onboard: drives the addressable RGB LED of board revision v1.1 (the initial revision used GPIO48)
- `J3.11` GPIO37 — reserved: GPIO35, GPIO36 and GPIO37 are used for the internal communication with the Octal SPI PSRAM of the module (N16R8): not available for external use
- `J3.12` GPIO36 — reserved: GPIO35, GPIO36 and GPIO37 are used for the internal communication with the Octal SPI PSRAM of the module (N16R8): not available for external use
- `J3.13` GPIO35 — reserved: GPIO35, GPIO36 and GPIO37 are used for the internal communication with the Octal SPI PSRAM of the module (N16R8): not available for external use
- `J3.14` GPIO0 — onboard: GPIO0 is the Boot button of the board and a strapping pin (module pack rule C-ESP-STRAP-IO0, boot rule C-ESP-BOOT-IO0)
- `J3.15` GPIO45 — unknown: GPIO45 is a strapping pin of the module (module pack rule C-ESP-STRAP-IO45); the board documents list it on the header and do not say that it is free for external circuits: nothing is assumed, and the pin is in no pool
- `J3.19` GPIO20 — onboard: USB D- (GPIO19) / D+ (GPIO20): wired to the board's second Micro-USB connector, the ESP32-S3 USB port (user guide; schematic sheet 2 `ESP USB`)
- `J3.20` GPIO19 — onboard: USB D- (GPIO19) / D+ (GPIO20): wired to the board's second Micro-USB connector, the ESP32-S3 USB port (user guide; schematic sheet 2 `ESP USB`)
- `J3.21` GND — reserved: ground: not a signal pin, in no pool
- `J3.22` GND — reserved: ground: not a signal pin, in no pool

## 4. Already on the board (not added again on the carrier)

- `BLK-DK-RESET` Reset button and RC circuit on EN (on the board)
- `BLK-DK-BOOT` Boot button on GPIO0 (on the board)
- `BLK-DK-USB-UART` Micro-USB port with USB-to-UART bridge on UART0 (on the board)
- `BLK-DK-USB` ESP32-S3 USB port on GPIO19 and GPIO20 (on the board)
- `BLK-DK-POWER` 5 V to 3.3 V regulator and power-on LED (on the board)
- `BLK-DK-RGB` Addressable RGB LED on GPIO38 (on the board)

## 5. What the carrier is to get

- **temperature sensor (I2C)** — feasible

## 6. Editable areas of the carrier

blocks that may be added: ["BLK-DK-PERIPHERAL"] allowed operations: ["add_component","connect","add_track"]

- `REG-CARRIER-FREE-1` [[0.0,0.0],[27.3,0.0],[27.3,100.0],[0.0,100.0]]
- `REG-CARRIER-FREE-2` [[27.3,0.0],[29.33,0.0],[29.33,37.1],[27.3,37.1]]
- `REG-CARRIER-FREE-3` [[29.33,0.0],[37.27,0.0],[37.27,36.227],[29.33,36.227]]
- `REG-CARRIER-FREE-4` [[37.27,0.0],[42.625,0.0],[42.625,37.1],[37.27,37.1]]
- `REG-CARRIER-FREE-5` [[42.625,0.0],[50.025,0.0],[50.025,36.227],[42.625,36.227]]
- `REG-CARRIER-FREE-6` [[50.025,0.0],[52.7,0.0],[52.7,37.1],[50.025,37.1]]
- `REG-CARRIER-FREE-7` [[52.7,0.0],[80.0,0.0],[80.0,100.0],[52.7,100.0]]

## 7. Unknown items and notes

- unknown: J1.13 (GPIO3): GPIO3 is a strapping pin of the module (module pack rule C-ESP-STRAP-IO3); the board documents list it on the header and do not say that it is free for external circuits: nothing is assumed, and the pin is in no pool
- unknown: J1.14 (GPIO46): GPIO46 is a strapping pin of the module (module pack rule C-ESP-STRAP-IO46); the board documents list it on the header and do not say that it is free for external circuits: nothing is assumed, and the pin is in no pool
- unknown: J3.15 (GPIO45): GPIO45 is a strapping pin of the module (module pack rule C-ESP-STRAP-IO45); the board documents list it on the header and do not say that it is free for external circuits: nothing is assumed, and the pin is in no pool
- unknown: rule C-DK-ANT-CLEARANCE (keepout): The DXF gives the antenna part of the module (18 x 6.3 mm, beyond the board's top edge) and that is the only zone this pack forbids. The user guide of the board says nothing about the clearance a carrier must keep beside or under it; the module's guideline (cut the base board away on both sides of the antenna and below it) is written for a module soldered to a base board, not for a board plugged into a carrier, and this pack does not apply it by guess. A person has to decide.
- 2 sliver(s) of free area narrower than 0.5 mm are not editable

## 8. Where the evidence is

The evidence of every region and every usable pin (document, version, page or section, quotation) is in the generated carrier pack; the board's own evidence comes from the locked documents its pack cites.
