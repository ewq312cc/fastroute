# ESP32-S3-WROOM-1-N8R8 模组级约束包 — 硬件约束知识包报告

> 这份报告回答四个问题：**什么不能动、什么必须保留、什么可以改、依据是什么**。每一条都列出来源文档、版本、页码/章节和原文摘录；“推导”“参考板观察”“待确认”不会被写成手册原文。

- 知识包: `espressif/esp32-s3/esp32-s3-wroom-1/n8r8` 0.1.0 · 抽取者: mixed
- 审查状态: **draft**（草稿：尚未经人工审查，不得作为最终依据）
- 器件: Espressif ESP32-S3  (DEV-ESP32S3), 7 个引脚
- 器件: Espressif ESP32-S3-WROOM-1-N8R8  (DEV-ESP32S3-WROOM1-N8R8), 41 个引脚

## 1. 不可修改项（固定）

- **C-S3C-BOOT-GPIO0** GPIO0 and GPIO46 both low select the Joint Download Boot mode — ESP32-S3 的启动模式脚 5: 电平 low → Joint Download Boot (UART or USB download); GPIO46 low as well
  - 依据: SRC-ESP32S3-DS: ESP32-S3 Series Datasheet v2.2 [官方手册], 第 33 页, 3.1 Chip Boot Mode Control — "GPIO0 and GPIO46 control the boot mode after the reset is released." (原文明确说明)
  - 依据: SRC-ESP32S3-DS: ESP32-S3 Series Datasheet v2.2 [官方手册], 第 33 页, Table 3-3. Chip Boot Mode Control — "Boot Mode GPIO0 GPIO46 SPI boot mode 1 Any value Joint download boot mode 2 0 0" (原文明确说明)
- **C-S3C-ELECTRICAL-SUPPLY** VDDA and VDD3P3 take 3.0 V to 3.6 V (3.3 V typical); the cumulative input current is at least 0.5 A — VDDA, VDD3P3 input voltage and cumulative input current (Table 5-2. Recommended Operating Conditions): 最小值 3.0 V; 典型值 3.3 V; 最大值 3.6 V; min current 0.5 A
  - 依据: SRC-ESP32S3-DS: ESP32-S3 Series Datasheet v2.2 [官方手册], 第 64 页, Table 5-2. Recommended Operating Conditions — "VDDA, VDD3P3 Recommended input voltage 3.0 3.3 3.6 V" (原文明确说明)
  - 依据: SRC-ESP32S3-DS: ESP32-S3 Series Datasheet v2.2 [官方手册], 第 64 页, Table 5-2. Recommended Operating Conditions — "Cumulative input current 0.5" (原文明确说明)
- **C-S3C-RESET** CHIP_PU low powers the chip off; it must not be left floating — ESP32-S3 的复位脚 4: 有效电平 low
  - 依据: SRC-ESP32S3-DS: ESP32-S3 Series Datasheet v2.2 [官方手册], 第 28 页, Table 2-9. Analog Pins — "Do not leave the CHIP_PU pin floating." (原文明确说明)
- **C-S3C-STRAP-GPIO0** GPIO0 is a strapping pin: weak pull-up (1) by default — ESP32-S3 的启动配置脚 5: 采样时刻 reset, 默认电平 high, 允许外部驱动
  - 依据: SRC-ESP32S3-DS: ESP32-S3 Series Datasheet v2.2 [官方手册], 第 32 页, Table 3-1. Default Configuration of Strapping Pins — "GPIO0 Weak pull-up 1" (原文明确说明)
- **C-S3C-STRAP-GPIO3** GPIO3 is a strapping pin without internal pull resistors — ESP32-S3 的启动配置脚 8: 采样时刻 reset, 默认电平 floating, 允许外部驱动
  - 适用条件: GPIO3 is used to select the JTAG signal source
  - 依据: SRC-ESP32S3-DS: ESP32-S3 Series Datasheet v2.2 [官方手册], 第 34 页, 3.3 JTAG Signal Source Control — "This pin does not have any internal pull resistors and the strapping value must be controlled by the external circuit that cannot be in a high impedance state." (原文明确说明)
- **C-S3C-STRAP-GPIO45** GPIO45 is a strapping pin: weak pull-down (0) by default — ESP32-S3 的启动配置脚 51: 采样时刻 reset, 默认电平 low, 允许外部驱动
  - 依据: SRC-ESP32S3-DS: ESP32-S3 Series Datasheet v2.2 [官方手册], 第 32 页, Table 3-1. Default Configuration of Strapping Pins — "GPIO45 Weak pull-down 0" (原文明确说明)
- **C-S3C-STRAP-GPIO46** GPIO46 is a strapping pin: weak pull-down (0) by default — ESP32-S3 的启动配置脚 52: 采样时刻 reset, 默认电平 low, 允许外部驱动
  - 依据: SRC-ESP32S3-DS: ESP32-S3 Series Datasheet v2.2 [官方手册], 第 32 页, Table 3-1. Default Configuration of Strapping Pins — "GPIO46 Weak pull-down 0" (原文明确说明)
- **C-ESP-RESERVED-PSRAM** IO35、IO36、IO37 接在模组内部的八线 PSRAM 上，不能另作他用 — 不得使用 ESP32-S3-WROOM-1-N8R8 28, 29, 30: connected to the Octal SPI PSRAM inside the module
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 3 页, Table 1-1. ESP32-S3-WROOM-1 Series Comparison — "ESP32-S3-WROOM-1-N8R8 8 MB (Quad SPI) 8 MB (Octal SPI)" (原文明确说明)
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 12 页, Table 3-1. Pin Definitions, note b — "pins IO35, IO36, and IO37 are connected to the Octal SPI PSRAM and are not available for other uses." (原文明确说明)
- **C-ESP-GND-1** 引脚 1 接地 — ESP32-S3-WROOM-1-N8R8 的引脚 1 必须接到网络 GND*
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 11 页, Table 3-1. Pin Definitions — "GND 1 P GND" (原文明确说明)
- **C-ESP-GND-40** 引脚 40 接地 — ESP32-S3-WROOM-1-N8R8 的引脚 40 必须接到网络 GND*
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 12 页, Table 3-1. Pin Definitions — "GND 40 P GND" (原文明确说明)
- **C-ESP-EN** EN 引脚低电平关闭芯片，不得悬空 — ESP32-S3-WROOM-1-N8R8 的复位脚 3: 有效电平 low
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 11 页, Table 3-1. Pin Definitions — "Low: off, the chip powers off." (原文明确说明)
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 11 页, Table 3-1. Pin Definitions — "Note: Do not leave the EN pin floating." (原文明确说明)
- **C-ESP-STRAP-IO0** GPIO0 是 strapping 引脚，默认弱上拉（1） — ESP32-S3-WROOM-1-N8R8 的启动配置脚 27: 采样时刻 reset, 默认电平 high, 允许外部驱动
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 13 页, Table 4-1. Default Configuration of Strapping Pins — "GPIO0 Weak pull-up 1" (原文明确说明)
  - 依据: SRC-ESP32S3-DS: ESP32-S3 Series Datasheet v2.2 [官方手册], 第 32 页, Table 3-1. Default Configuration of Strapping Pins — "GPIO0 Weak pull-up 1" (原文明确说明)
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 13 页, 4 Boot Configurations — "the strapping pin voltage levels can also be controlled by the host MCU." (原文明确说明)
- **C-ESP-STRAP-IO46** GPIO46 是 strapping 引脚，默认弱下拉（0） — ESP32-S3-WROOM-1-N8R8 的启动配置脚 16: 采样时刻 reset, 默认电平 low, 允许外部驱动
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 13 页, Table 4-1. Default Configuration of Strapping Pins — "GPIO46 Weak pull-down 0" (原文明确说明)
- **C-ESP-STRAP-IO45** GPIO45 是 strapping 引脚，默认弱下拉（0） — ESP32-S3-WROOM-1-N8R8 的启动配置脚 26: 采样时刻 reset, 默认电平 low, 允许外部驱动
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 13 页, Table 4-1. Default Configuration of Strapping Pins — "GPIO45 Weak pull-down 0" (原文明确说明)
- **C-ESP-STRAP-IO3** GPIO3 是 strapping 引脚，没有内部上下拉 — ESP32-S3-WROOM-1-N8R8 的启动配置脚 15: 采样时刻 reset, 默认电平 floating, 允许外部驱动
  - 适用条件: GPIO3 is used to select the JTAG signal source (Table 4-5: EFUSE_STRAP_JTAG_SEL = 1)
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 15 页, 4.4 JTAG Signal Source Control — "This pin does not have any internal pull resistors and the strapping value must be controlled by the external circuit that cannot be in a high impedance state." (原文明确说明)
- **C-ESP-BOOT-IO0** GPIO0 与 GPIO46 同为 0 时进入联合下载启动模式 — ESP32-S3-WROOM-1-N8R8 的启动模式脚 27: 电平 low → Joint Download Boot (UART or USB download)
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 14 页, 4.1 Chip Boot Mode Control — "GPIO0 and GPIO46 control the boot mode after the reset is released." (原文明确说明)
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 14 页, Table 4-3. Chip Boot Mode Control — "SPI Boot 1 Any value Joint Download Boot 2 0 0" (原文明确说明)
- **C-ESP-ELECTRICAL-VDD33** 供电电压 3.0 V 至 3.6 V，外部电源至少能提供 0.5 A — VDD33 supply voltage and current (recommended operating conditions (Table 6-2)): 最小值 3.0 V; 典型值 3.3 V; 最大值 3.6 V; min current 0.5 A
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 27 页, Table 6-2. Recommended Operating Conditions — "VDD33 Power supply voltage 3.0 3.3 3.6 V" (原文明确说明)
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 27 页, Table 6-2. Recommended Operating Conditions — "Current delivered by external power supply 0.5" (原文明确说明)

## 2. 必需外设与必须保留项

（没有）

## 3. 可配置项、可选项与推导项

[可配置]
- **C-ESP-NEED-RESET** 建议在 EN 上加 RC 延时电路（通常 10 kΩ + 1 µF，需按实际上电时序调整） — 必须有外设块 EN 引脚 RC 延时电路（通常 10 kΩ + 1 µF）
  - 严重级别: 警告（不使校验失败）
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 41 页, 9 Peripheral Schematics — "The recommended setting for the RC delay circuit is usually R = 10 kΩ and C = 1 µF." (原文明确说明)
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 41 页, 9 Peripheral Schematics — "However, specific parameters should be adjusted based on the power-up timing of the module and the power-up and reset sequence timing of the chip." (原文明确说明)
  - 依据: SRC-ESP32S3-HDG-SCH: ESP32-S3 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], Chip Power-up and Reset Timing — "The recommended setting for the RC delay circuit is usually R = 10 kΩ and C = 1 μF." (原文明确说明)
  - 依据: SRC-ESP32S3-HDG-SCH: ESP32-S3 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], Chip Power-up and Reset Timing — "However, specific parameters should be adjusted based on the characteristics of the actual power supply and the power-up and reset timing of the chip." (原文明确说明)
[推导]
- **C-ESP-NEED-BOOT** 下载模式需要把 GPIO0 拉低：板上应有按键、跳线或下载工具驱动的入口 — 必须有外设块 GPIO0 下拉入口（按键或跳线），用于进入下载模式
  - 推导自: C-ESP-BOOT-IO0 (这是推导，不是手册原文)
  - 严重级别: 警告（不使校验失败）
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 14 页, Table 4-3. Chip Boot Mode Control — "SPI Boot 1 Any value Joint Download Boot 2 0 0" (由多条规则推导)
[可配置]
- **C-ESP-NEED-UART** 通常用 UART0 下载和打印日志，建议留出 TXD0/RXD0 接口 — 必须有外设块 UART0 下载/日志接口（连接器接在 TXD0 与 RXD0 上）
  - 严重级别: 警告（不使校验失败）
  - 依据: SRC-ESP32S3-HDG-SCH: ESP32-S3 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], UART — "Usually, UART0 is used as the serial port for download and log printing." (原文明确说明)
[可配置]
- **C-ESP-NEED-POWER** 电源入口至少 10 µF 电容（发射时电流突增会使电源轨塌陷） — 必须有外设块 电源入口大电容（至少 10 µF）
  - 依据: SRC-ESP32S3-HDG-SCH: ESP32-S3 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], Power Supply — "It is suggested to add an ESD protection diode and at least 10 μF capacitor at the main power entrance (where the external power supply enters the PCB)." (原文明确说明)
- **BLK-ESP-POWER** 电源入口大电容（至少 10 µF） (power_decoupling)
  - bulk: 1 × capacitor (最小值 10000 nF)
  - 连接 2 → *
  - 依据: SRC-ESP32S3-HDG-SCH: ESP32-S3 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], Power Supply — "It is suggested to add an ESD protection diode and at least 10 μF capacitor at the main power entrance (where the external power supply enters the PCB)." (原文明确说明)
- **BLK-ESP-RESET** EN 引脚 RC 延时电路（通常 10 kΩ + 1 µF） (reset_circuit)
  - pull_up: 1 × resistor (数值 10000 Ω)
  - delay: 1 × capacitor (最小值 1000 nF)
  - 连接 3 → *
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 41 页, 9 Peripheral Schematics — "The recommended setting for the RC delay circuit is usually R = 10 kΩ and C = 1 µF." (原文明确说明)
  - 依据: SRC-ESP32S3-HDG-SCH: ESP32-S3 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], Chip Power-up and Reset Timing — "The recommended setting for the RC delay circuit is usually R = 10 kΩ and C = 1 μF." (原文明确说明)
- **BLK-ESP-BOOT** GPIO0 下拉入口（按键或跳线），用于进入下载模式 (boot_button)
  - button: 1 × switch
  - 连接 27 → *
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 14 页, Table 4-3. Chip Boot Mode Control — "SPI Boot 1 Any value Joint Download Boot 2 0 0" (由多条规则推导)
- **BLK-ESP-UART-DL** UART0 下载/日志接口（连接器接在 TXD0 与 RXD0 上） (programming_interface)
  - header: 1 × connector
  - 连接 37 → *
  - 连接 36 → *
  - 依据: SRC-ESP32S3-HDG-SCH: ESP32-S3 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], UART — "Usually, UART0 is used as the serial port for download and log printing." (原文明确说明)

## 4. AI 可编辑区域（白名单）

默认所有原理图、PCB、引脚和机械对象只读；只有下面列出的内容可以被 AI 修改，其余一律不得改动。

（没有 base_design：没有任何东西可由 AI 修改）

## 5. 高速 / 机械 / 布线约束

[configurable]
- **C-ESP-USB** USB 差分对：90 Ω ±10%，平行等长走线 — 接口网络 USB_DP, USB_DM, USB_D+, USB_D- (差分) (阻抗 90 Ω; impedance tolerance 10 %)
  - 依据: SRC-ESP32S3-HDG-PCB: ESP32-S3 Hardware Design Guidelines - PCB Layout Design latest-2026-11-16 [官方设计指南], USB — "Use differential pairs and route them in parallel at equal lengths. Maintain a differential pair impedance of 90 Ω with a tolerance of ±10%." (原文明确说明)
  - 依据: SRC-ESP32S3-HDG-SCH: ESP32-S3 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], USB — "GPIO19 and GPIO20 can be used as D- and D+ of USB respectively." (原文明确说明)
[required]
- **C-ESP-ANT-KEEPOUT** 模组 PCB 天线的禁布区：此区域内不得有走线、过孔、元件和铜皮 — 区域 REG-ESP-ANT 天线禁布区（模组坐标系，原点在模组 18 x 25.5 mm 本体的中心，天线在 -y 端；按指南图 Keepout Zone ... (Antenna feed point on the right) 的标注读取，待人工确认） 内禁止: track, via, component, zone
  - 依据: SRC-ESP32S3-WROOM1-DS: ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet v1.8 [官方手册], 第 10 页, Figure 3-1. Pin Layout (Top View) — "To learn more about the keepout zone for module’s antenna on the base board, please refer to ESP32-S3 Hardware Design Guidelines > Section General Principles of PCB Layout for Modules." (原文明确说明)
  - 依据: SRC-ESP32S3-HDG-PCB: ESP32-S3 Hardware Design Guidelines - PCB Layout Design latest-2026-11-16 [官方设计指南], General Principles of PCB Layout for Modules — "Then cut off the base board on both sides of the antenna and below it to minimize the impact of the base board material on the PCB antenna and provide a sufficiently large clearance area for the PCB antenna." (原文明确说明)
- **REG-ESP-ANT** 天线禁布区（模组坐标系，原点在模组 18 x 25.5 mm 本体的中心，天线在 -y 端；按指南图 Keepout Zone ... (Antenna feed point on the right) 的标注读取，待人工确认） — 天线禁区, x -24..11 mm, y -13.75..-6.75 mm (4 个顶点), 层: all [fixed]
  - 依据: SRC-ESP32S3-HDG-PCB: ESP32-S3 Hardware Design Guidelines - PCB Layout Design latest-2026-11-16 [官方设计指南], General Principles of PCB Layout for Modules, Keepout Zone for ESP32-S3 Module's Antenna (Antenna feed point on the right) — "Keepout Zone for ESP32-S3 Module’s Antenna (Antenna feed point on the right)" (待确认)

## 6. 冲突与缺口

- **C-ESP-EN-RC-VALUES** 资料未说明: 必须有外设块 EN 引脚 RC 延时电路（通常 10 kΩ + 1 µF）
  - 缺口: Figure 9-1 of the module datasheet v1.8 (page 41) shows R1 and C2 of the EN circuit as "TBD". The text of the same page and the Hardware Design Guidelines say "usually R = 10 kΩ and C = 1 µF" and tell the designer to adjust the values to the power-up timing of the module and the chip (rule C-ESP-NEED-RESET keeps the usual values as a recommendation). The final values depend on the supply ramp of the project, which no document gives: the project has to decide them and test the power-up.
- **C-ESP-USB-SKEW** 资料未说明: 接口网络 USB_DP, USB_DM, USB_D+, USB_D- (差分) (偏斜 未知)
  - 缺口: The PCB Layout Design guideline says to route the pair "in parallel at equal lengths" and gives no number. No document read for this pack gives a skew limit in millimetres; do not use a typical value.
- **C-ESP-USB-GAP** 资料未说明: 接口网络 USB_DP, USB_DM, USB_D+, USB_D- (差分) (间距 未知)
  - 缺口: The gap between the two traces depends on the stack-up and the trace width; the documents read give the 90 Ω target but no spacing. The stack-up of the board is needed (the plugin's impedance tool can derive widths from it).
- 待确认的依据: `$.regions[0].evidence[0]`

## 7. 来源列表

| 编号 | 文档 | 版本 | 级别 | 获取日期 | SHA-256 |
|---|---|---|---|---|---|
| SRC-ESP32S3-WROOM1-DS / DOC-ESP32S3-WROOM1-DS | ESP32-S3-WROOM-1 & ESP32-S3-WROOM-1U Datasheet (https://www.espressif.com/sites/default/files/documentation/esp32-s3-wroom-1_wroom-1u_datasheet_en.pdf) | v1.8 | 官方手册 | 2026-11-16 | `27d71971da07c280` |
| SRC-ESP32S3-DS / DOC-ESP32S3-DS | ESP32-S3 Series Datasheet (https://www.espressif.com/sites/default/files/documentation/esp32-s3_datasheet_en.pdf) | v2.2 | 官方手册 | 2026-11-16 | `2d5a7cb7fd559d8d` |
| SRC-ESP32S3-HDG-SCH / DOC-ESP32S3-HDG-SCH | ESP32-S3 Hardware Design Guidelines - Schematic Checklist (https://docs.espressif.com/projects/esp-hardware-design-guidelines/en/latest/esp32s3/schematic-checklist.html) | latest-2026-11-16 | 官方设计指南 | 2026-11-16 | `6cc621dfa1caec2c` |
| SRC-ESP32S3-HDG-PCB / DOC-ESP32S3-HDG-PCB | ESP32-S3 Hardware Design Guidelines - PCB Layout Design (https://docs.espressif.com/projects/esp-hardware-design-guidelines/en/latest/esp32s3/pcb-layout-design.html) | latest-2026-11-16 | 官方设计指南 | 2026-11-16 | `92318f21920eb2fc` |

回跳原文：`hcp-manager evidence verify <pack>` 检查每条引用的文档、版本、摘要和页码上的原文；`hcp-manager search "…" --device <型号>` 找到同一版本的原文块。
