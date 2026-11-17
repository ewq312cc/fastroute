# ESP32-S3-DevKitC-1 v1.1 (ESP32-S3-WROOM-1-N16R8): board pack — 硬件约束知识包报告

> 这份报告回答四个问题：**什么不能动、什么必须保留、什么可以改、依据是什么**。每一条都列出来源文档、版本、页码/章节和原文摘录；“推导”“参考板观察”“待确认”不会被写成手册原文。

- 知识包: `espressif/esp32-s3/esp32-s3-devkitc-1/n16r8-v1.1` 0.1.0 · 抽取者: mixed
- 审查状态: **draft**（草稿：尚未经人工审查，不得作为最终依据）
- 器件: Espressif ESP32-S3-WROOM-1-N16R8  (DEV-ESP32S3-WROOM1-N16R8), 41 个引脚
- 器件: Espressif ESP32-S3-DevKitC-1-N16R8 v1.1 (DEV-DEVKITC1-N16R8-V11), 44 个引脚

## 1. 不可修改项（固定）

- **C-ESP-BOOT-IO0** GPIO0 与 GPIO46 同为 0 时进入联合下载启动模式 — ESP32-S3-WROOM-1-N16R8 的启动模式脚 27: 电平 low → Joint Download Boot (UART or USB download)
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 14 页, 4.1 Chip Boot Mode Control — "GPIO0 and GPIO46 control the boot mode after the reset is released." (原文明确说明)
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 14 页, Table 4-3. Chip Boot Mode Control — "SPI Boot 1 Any value Joint Download Boot 2 0 0" (原文明确说明)
- **C-ESP-EN** EN 引脚低电平关闭芯片，不得悬空 — ESP32-S3-WROOM-1-N16R8 的复位脚 3: 有效电平 low
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 11 页, Table 3-1. Pin Definitions — "Low: off, the chip powers off." (原文明确说明)
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 11 页, Table 3-1. Pin Definitions — "Note: Do not leave the EN pin floating." (原文明确说明)
- **C-ESP-RESERVED-PSRAM** IO35、IO36、IO37 接在模组内部的八线 PSRAM 上，不能另作他用 — 不得使用 ESP32-S3-WROOM-1-N16R8 28, 29, 30: connected to the Octal SPI PSRAM inside the module
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 3 页, Table 1-1. ESP32-S3-WROOM-1 Series Comparison — "ESP32-S3-WROOM-1-N16R8 16 MB (Quad SPI) 8 MB (Octal SPI)" (原文明确说明)
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 12 页, Table 3-1. Pin Definitions, note b — "pins IO35, IO36, and IO37 are connected to the Octal SPI PSRAM and are not available for other uses." (原文明确说明)
- **C-ESP-STRAP-IO0** GPIO0 是 strapping 引脚，默认弱上拉（1） — ESP32-S3-WROOM-1-N16R8 的启动配置脚 27: 采样时刻 reset, 默认电平 high, 允许外部驱动
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 13 页, Table 4-1. Default Configuration of Strapping Pins — "GPIO0 Weak pull-up 1" (原文明确说明)
  - 依据: SRC-ESP32S3-DS: ESP32-S3 Series Datasheet v2.2 [官方手册], 第 32 页, Table 3-1. Default Configuration of Strapping Pins — "GPIO0 Weak pull-up 1" (原文明确说明)
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 13 页, 4 Boot Configurations — "the strapping pin voltage levels can also be controlled by the host MCU." (原文明确说明)
- **C-ESP-STRAP-IO3** GPIO3 是 strapping 引脚，没有内部上下拉 — ESP32-S3-WROOM-1-N16R8 的启动配置脚 15: 采样时刻 reset, 默认电平 floating, 允许外部驱动
  - 适用条件: GPIO3 is used to select the JTAG signal source (Table 4-5: EFUSE_STRAP_JTAG_SEL = 1)
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 15 页, 4.4 JTAG Signal Source Control — "This pin does not have any internal pull resistors and the strapping value must be controlled by the external circuit that cannot be in a high impedance state." (原文明确说明)
- **C-ESP-STRAP-IO45** GPIO45 是 strapping 引脚，默认弱下拉（0） — ESP32-S3-WROOM-1-N16R8 的启动配置脚 26: 采样时刻 reset, 默认电平 low, 允许外部驱动
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 13 页, Table 4-1. Default Configuration of Strapping Pins — "GPIO45 Weak pull-down 0" (原文明确说明)
- **C-ESP-STRAP-IO46** GPIO46 是 strapping 引脚，默认弱下拉（0） — ESP32-S3-WROOM-1-N16R8 的启动配置脚 16: 采样时刻 reset, 默认电平 low, 允许外部驱动
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 13 页, Table 4-1. Default Configuration of Strapping Pins — "GPIO46 Weak pull-down 0" (原文明确说明)
- **C-DK-RSVD-PSRAM** GPIO35, GPIO36 and GPIO37 (J3.13, J3.12, J3.11) are used for the Octal SPI flash/PSRAM of the module and are not available for external use — 不得使用 ESP32-S3-DevKitC-1-N16R8 J3.11, J3.12, J3.13: used for the internal communication between the ESP32-S3 and the Octal SPI memory of the module
  - 依据: SRC-DK-UG: ESP32-S3-DevKitC-1 v1.1 User Guide latest-2026-11-17 [官方开发板], Getting Started, Description of Components — "For boards with Octal SPI flash/PSRAM memory embedded ESP32-S3-WROOM-1/1U modules, and boards with ESP32-S3-WROOM-2 modules, the pins GPIO35, GPIO36 and GPIO37 are used for the internal communication between ESP32-S3 and SPI flash/PSRAM memory, thus not available for external use." (原文明确说明)
  - 依据: SRC-DK-UG: ESP32-S3-DevKitC-1 v1.1 User Guide latest-2026-11-17 [官方开发板], Contents and Packaging, Ordering Information — "ESP32-S3-DevKitC-1-N16R8 ESP32-S3-WROOM-1 16 MB 8 MB PCB on-board antenna" (原文明确说明)
- **BLK-DK-RESET** Reset button and RC circuit on EN (on the board) (reset_circuit)
  - 依据: SRC-DK-UG: ESP32-S3-DevKitC-1 v1.1 User Guide latest-2026-11-17 [官方开发板], Getting Started, Description of Components — "Reset Button" (原文明确说明)
  - 依据: SRC-DK-UG: ESP32-S3-DevKitC-1 v1.1 User Guide latest-2026-11-17 [官方开发板], Getting Started, Description of Components — "Press this button to restart the system." (原文明确说明)
- **BLK-DK-BOOT** Boot button on GPIO0 (on the board) (boot_button)
  - 依据: SRC-DK-UG: ESP32-S3-DevKitC-1 v1.1 User Guide latest-2026-11-17 [官方开发板], Getting Started, Description of Components — "Boot Button" (原文明确说明)
  - 依据: SRC-DK-UG: ESP32-S3-DevKitC-1 v1.1 User Guide latest-2026-11-17 [官方开发板], Getting Started, Description of Components — "Holding down Boot and then pressing Reset initiates Firmware Download mode for downloading firmware through the serial port." (原文明确说明)
- **BLK-DK-USB-UART** Micro-USB port with USB-to-UART bridge on UART0 (on the board) (programming_interface)
  - 依据: SRC-DK-UG: ESP32-S3-DevKitC-1 v1.1 User Guide latest-2026-11-17 [官方开发板], Getting Started, Description of Components — "A Micro-USB port used for power supply to the board, for flashing applications to the chip, as well as for communication with the chip via the on-board USB-to-UART bridge." (原文明确说明)
  - 依据: SRC-DK-UG: ESP32-S3-DevKitC-1 v1.1 User Guide latest-2026-11-17 [官方开发板], Getting Started, Description of Components — "Single USB-to-UART bridge chip provides transfer rates up to 3 Mbps." (原文明确说明)
- **BLK-DK-USB** ESP32-S3 USB port on GPIO19 and GPIO20 (on the board) (usb)
  - 依据: SRC-DK-UG: ESP32-S3-DevKitC-1 v1.1 User Guide latest-2026-11-17 [官方开发板], Getting Started, Description of Components — "ESP32-S3 full-speed USB OTG interface, compliant with the USB 1.1 specification." (原文明确说明)
- **BLK-DK-POWER** 5 V to 3.3 V regulator and power-on LED (on the board) (power_decoupling)
  - 依据: SRC-DK-UG: ESP32-S3-DevKitC-1 v1.1 User Guide latest-2026-11-17 [官方开发板], Getting Started, Description of Components — "Power regulator that converts a 5 V supply into a 3.3 V output." (原文明确说明)
  - 依据: SRC-DK-UG: ESP32-S3-DevKitC-1 v1.1 User Guide latest-2026-11-17 [官方开发板], Getting Started, Description of Components — "Turns on when the USB power is connected to the board." (原文明确说明)
- **BLK-DK-RGB** Addressable RGB LED on GPIO38 (on the board) (generic)
  - 依据: SRC-DK-UG: ESP32-S3-DevKitC-1 v1.1 User Guide latest-2026-11-17 [官方开发板], Getting Started, Description of Components — "Addressable RGB LED, driven by GPIO38." (原文明确说明)

## 2. 必需外设与必须保留项

（没有）

## 3. 可配置项、可选项与推导项

（没有）

## 4. AI 可编辑区域（白名单）

默认所有原理图、PCB、引脚和机械对象只读；只有下面列出的内容可以被 AI 修改，其余一律不得改动。

（没有 base_design：没有任何东西可由 AI 修改）

## 5. 高速 / 机械 / 布线约束

- **REG-DK-ANT** The antenna part of the module (PCB antenna, `MINI-ANT-TYPEB` in the DXF): 18 x 6.3 mm that stick out beyond the top edge of the board; nothing of a carrier may be here. The wider RF clearance is unknown (C-DK-ANT-CLEARANCE) — 天线禁区, x 3.75..21.75 mm, y 62.84..69.15 mm (4 个顶点) [fixed]
  - 依据: SRC-DK-DXF: ESP32-S3-DevKitC-1 Dimensions source file V1.1 (DXF, 2022-04-29) V1.1_20220429 [官方开发板], DXF text MINI-ANT-TYPEB — "1 MINI-ANT-TYPEB" (参考板观察)
  - 依据: SRC-DK-DXF: ESP32-S3-DevKitC-1 Dimensions source file V1.1 (DXF, 2022-04-29) V1.1_20220429 [官方开发板], DXF layer TOP, antenna outline — "8 TOP 100 AcDbPolyline 90 5 70 1 43 0.0 10 3.75 20 69.15" (参考板观察)
- **REG-DK-OUTLINE** Outline of the board, the bounding rectangle of the BOARD_OUTLINE layer of the DXF: 25.4 x 62.865 mm (corners are rounded, which this pack does not model). The drawing's own dimension texts say 25.40mm and 62.74mm; the outline layer measures 62.865 mm in length, 0.125 mm more, and the larger value is used — 板框, x 0..25.4 mm, y 0..62.865 mm (4 个顶点) [fixed]
  - 依据: SRC-DK-DXF: ESP32-S3-DevKitC-1 Dimensions source file V1.1 (DXF, 2022-04-29) V1.1_20220429 [官方开发板], DXF layer BOARD_OUTLINE — "8 BOARD_OUTLINE 100 AcDbPolyline 90 2 70 0 43 0.127 10 0.0 20 0.5" (参考板观察)
  - 依据: SRC-DK-DXF: ESP32-S3-DevKitC-1 Dimensions source file V1.1 (DXF, 2022-04-29) V1.1_20220429 [官方开发板], DXF dimension text — "1 25.40mm" (参考板观察)
  - 依据: SRC-DK-DXF: ESP32-S3-DevKitC-1 Dimensions source file V1.1 (DXF, 2022-04-29) V1.1_20220429 [官方开发板], DXF dimension text — "1 62.74mm" (参考板观察)
- **REG-DK-USB-J2** Micro-USB connector J2 (USB-to-UART port): the 0.873 mm of its shell that stick out beyond the bottom edge of the board — 连接器, x 2.03..9.97 mm, y -0.873..0 mm (4 个顶点) [fixed]
  - 依据: SRC-DK-DXF: ESP32-S3-DevKitC-1 Dimensions source file V1.1 (DXF, 2022-04-29) V1.1_20220429 [官方开发板], DXF layer 2DLINES_1, connector drawing — "8 2DLINES_1 100 AcDbPolyline 90 2 70 0 43 0.1016 10 2.7 20 -0.22267 10 2.7 20 -0.8726700000000001" (参考板观察)
- **REG-DK-USB-J4** Micro-USB connector J4 (ESP32-S3 USB port): the 0.873 mm of its shell that stick out beyond the bottom edge of the board — 连接器, x 15.325..22.725 mm, y -0.873..0 mm (4 个顶点) [fixed]
  - 依据: SRC-DK-DXF: ESP32-S3-DevKitC-1 Dimensions source file V1.1 (DXF, 2022-04-29) V1.1_20220429 [官方开发板], DXF layer 2DLINES_1, connector drawing — "8 2DLINES_1 100 AcDbPolyline 90 2 70 0 43 0.1016 10 16.1 20 -0.22267 10 16.1 20 -0.8726700000000001" (参考板观察)

## 6. 冲突与缺口

- **C-DK-ANT-CLEARANCE** 资料未说明: 区域 REG-DK-ANT The antenna part of the module (PCB antenna, `MINI-ANT-TYPEB` in the DXF): 18 x 6.3 mm that stick out beyond the top edge of the board; nothing of a carrier may be here. The wider RF clearance is unknown (C-DK-ANT-CLEARANCE) 内禁止: track, via, component, zone
  - 缺口: The DXF gives the antenna part of the module (18 x 6.3 mm, beyond the board's top edge) and that is the only zone this pack forbids. The user guide of the board says nothing about the clearance a carrier must keep beside or under it; the module's guideline (cut the base board away on both sides of the antenna and below it) is written for a module soldered to a base board, not for a board plugged into a carrier, and this pack does not apply it by guess. A person has to decide.

## 7. 来源列表

| 编号 | 文档 | 版本 | 级别 | 获取日期 | SHA-256 |
|---|---|---|---|---|---|
| SRC-DK-UG / DOC-DEVKITC1-UG-V11 | ESP32-S3-DevKitC-1 v1.1 User Guide (https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s3/esp32-s3-devkitc-1/user_guide_v1.1.html) | latest-2026-11-17 | 官方开发板 | 2026-11-17 | `5120d2d6e8c55bb9` |
| SRC-DK-SCH / DOC-DEVKITC1-SCH-V11 | ESP32-S3-DevKitC-1 Board Schematic V1.1 (2022-11-30) (https://dl.espressif.com/dl/schematics/SCH_ESP32-S3-DevKitC-1_V1.1_20221130.pdf) | V1.1_20221130 | 官方开发板 | 2026-11-17 | `f116b5d9f1fdce12` |
| SRC-DK-DXF / DOC-DEVKITC1-DXF-V11 | ESP32-S3-DevKitC-1 Dimensions source file V1.1 (DXF, 2022-04-29) (https://dl.espressif.com/dl/schematics/esp_idf/DXF_ESP32-S3-DevKitC-1_V1.1_20220429.dxf) | V1.1_20220429 | 官方开发板 | 2026-11-17 | `02c88dea5acf89ef` |
| SRC-ESP32S3-DS / DOC-ESP32S3-DS | ESP32-S3 Series Datasheet (https://www.espressif.com/sites/default/files/documentation/esp32-s3_datasheet_en.pdf) | v2.2 | 官方手册 | 2026-11-16 | `2d5a7cb7fd559d8d` |
| SRC-ESP32S3-WROOM1-DS / DOC-ESP32S3-WROOM1-DS | ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet (https://www.espressif.com/sites/default/files/documentation/esp32-s3-wroom-1_wroom-1u_datasheet_en.pdf) | v1.8 | 官方手册 | 2026-11-16 | `27d71971da07c280` |

回跳原文：`hcp-manager evidence verify <pack>` 检查每条引用的文档、版本、摘要和页码上的原文；`hcp-manager search "…" --device <型号>` 找到同一版本的原文块。
