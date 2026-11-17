# 开发板嵌入报告 espressif/esp32-s3/esp32-s3-devkitc-1/n16r8-v1.1

- 开发板: espressif/esp32-s3/esp32-s3-devkitc-1/n16r8-v1.1 0.1.0 (硬件版本 v1.1), 包摘要 `46ac681cd7f3f275`
- 载板: sensor-carrier
- 位置: U1 (旋转 0°)
- 结论: **needs_attention**

## 一、开发板固定部分（不可改写）

开发板本体、排针、板框和下列禁区在载板上的位置固定，AI 不得移动、重布或覆盖：

- `REG-BOARD-BODY` (module_body) [[27.3,37.1],[52.7,37.1],[52.7,99.965],[27.3,99.965]]
- `REG-DK-ANT` (antenna_keepout) [[31.05,99.94],[49.05,99.94],[49.05,106.25],[31.05,106.25]]
- `REG-DK-USB-J2` (connector) [[29.33,36.227],[37.27,36.227],[37.27,37.1],[29.33,37.1]]
- `REG-DK-USB-J4` (connector) [[42.625,36.227],[50.025,36.227],[50.025,37.1],[42.625,37.1]]

## 二、可用排针

24 个（开发板对外引出且有证据）: J1.10, J1.11, J1.12, J1.15, J1.16, J1.17, J1.18, J1.19, J1.20, J1.4, J1.5, J1.6, J1.7, J1.8, J1.9, J3.16, J3.17, J3.18, J3.4, J3.5, J3.6, J3.7, J3.8, J3.9

## 三、不可用或未知的排针

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

## 四、开发板已板载（载板不得重复添加）

- `BLK-DK-RESET` Reset button and RC circuit on EN (on the board)
- `BLK-DK-BOOT` Boot button on GPIO0 (on the board)
- `BLK-DK-USB-UART` Micro-USB port with USB-to-UART bridge on UART0 (on the board)
- `BLK-DK-USB` ESP32-S3 USB port on GPIO19 and GPIO20 (on the board)
- `BLK-DK-POWER` 5 V to 3.3 V regulator and power-on LED (on the board)
- `BLK-DK-RGB` Addressable RGB LED on GPIO38 (on the board)

## 五、载板待连接的外设

- **temperature sensor (I2C)** — feasible

## 六、载板可编辑区域

可添加的块: ["BLK-DK-PERIPHERAL"] 允许的操作: ["add_component","connect","add_track"]

- `REG-CARRIER-FREE-1` [[0.0,0.0],[27.3,0.0],[27.3,100.0],[0.0,100.0]]
- `REG-CARRIER-FREE-2` [[27.3,0.0],[29.33,0.0],[29.33,37.1],[27.3,37.1]]
- `REG-CARRIER-FREE-3` [[29.33,0.0],[37.27,0.0],[37.27,36.227],[29.33,36.227]]
- `REG-CARRIER-FREE-4` [[37.27,0.0],[42.625,0.0],[42.625,37.1],[37.27,37.1]]
- `REG-CARRIER-FREE-5` [[42.625,0.0],[50.025,0.0],[50.025,36.227],[42.625,36.227]]
- `REG-CARRIER-FREE-6` [[50.025,0.0],[52.7,0.0],[52.7,37.1],[50.025,37.1]]
- `REG-CARRIER-FREE-7` [[52.7,0.0],[80.0,0.0],[80.0,100.0],[52.7,100.0]]

## 七、未知项与备注

- 未知: J1.13 (GPIO3): GPIO3 is a strapping pin of the module (module pack rule C-ESP-STRAP-IO3); the board documents list it on the header and do not say that it is free for external circuits: nothing is assumed, and the pin is in no pool
- 未知: J1.14 (GPIO46): GPIO46 is a strapping pin of the module (module pack rule C-ESP-STRAP-IO46); the board documents list it on the header and do not say that it is free for external circuits: nothing is assumed, and the pin is in no pool
- 未知: J3.15 (GPIO45): GPIO45 is a strapping pin of the module (module pack rule C-ESP-STRAP-IO45); the board documents list it on the header and do not say that it is free for external circuits: nothing is assumed, and the pin is in no pool
- 未知: rule C-DK-ANT-CLEARANCE (keepout): The DXF gives the antenna part of the module (18 x 6.3 mm, beyond the board's top edge) and that is the only zone this pack forbids. The user guide of the board says nothing about the clearance a carrier must keep beside or under it; the module's guideline (cut the base board away on both sides of the antenna and below it) is written for a module soldered to a base board, not for a board plugged into a carrier, and this pack does not apply it by guess. A person has to decide.
- 2 sliver(s) of free area narrower than 0.5 mm are not editable

## 八、证据位置

每个区域和每个可用排针的证据（文档、版本、页码或章节、原文引用）都在生成的载板知识包中，开发板自身的证据来自开发板包所引用的锁定文档。
