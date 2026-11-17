# ESP32-C3 (QFN32): chip-level constraints — 硬件约束知识包报告

> 这份报告回答四个问题：**什么不能动、什么必须保留、什么可以改、依据是什么**。每一条都列出来源文档、版本、页码/章节和原文摘录；“推导”“参考板观察”“待确认”不会被写成手册原文。

- 知识包: `espressif/esp32-c3/esp32-c3/qfn32` 0.1.0 · 抽取者: mixed
- 审查状态: **draft**（草稿：尚未经人工审查，不得作为最终依据）
- 器件: Espressif ESP32-C3  (DEV-ESP32C3-QFN32), 6 个引脚

## 1. 不可修改项（固定）

- **C-C3C-STRAP-GPIO2** GPIO2 is a strapping pin: floating by default (pull it up: it avoids glitches) — ESP32-C3 的启动配置脚 6: 采样时刻 reset, 默认电平 floating, 允许外部驱动
  - 依据: SRC-ESP32C3-DS: ESP32-C3 Series Datasheet v2.5 [官方手册], 第 30 页, Table 3-1. Default Configuration of Strapping Pins — "GPIO2 Floating" (原文明确说明)
- **C-C3C-STRAP-GPIO8** GPIO8 is a strapping pin: floating by default — ESP32-C3 的启动配置脚 14: 采样时刻 reset, 默认电平 floating, 允许外部驱动
  - 依据: SRC-ESP32C3-DS: ESP32-C3 Series Datasheet v2.5 [官方手册], 第 30 页, Table 3-1. Default Configuration of Strapping Pins — "GPIO8 Floating" (原文明确说明)
- **C-C3C-STRAP-GPIO9** GPIO9 is a strapping pin: weak pull-up (1) by default — ESP32-C3 的启动配置脚 15: 采样时刻 reset, 默认电平 high, 允许外部驱动
  - 依据: SRC-ESP32C3-DS: ESP32-C3 Series Datasheet v2.5 [官方手册], 第 30 页, Table 3-1. Default Configuration of Strapping Pins — "GPIO9 Weak pull-up 1" (原文明确说明)
- **C-C3C-BOOT-GPIO9** GPIO8 high and GPIO9 low select the Joint Download Boot mode — ESP32-C3 的启动模式脚 15: 电平 low → Joint download boot (USB-Serial-JTAG or UART download); GPIO8 must be high
  - 依据: SRC-ESP32C3-DS: ESP32-C3 Series Datasheet v2.5 [官方手册], 第 31 页, 3.1 Chip Boot Mode Control — "GPIO2, GPIO8, and GPIO9 control the boot mode after the reset is released." (原文明确说明)
  - 依据: SRC-ESP32C3-DS: ESP32-C3 Series Datasheet v2.5 [官方手册], 第 31 页, Table 3-3. Chip Boot Mode Control — "SPI boot mode 1 Any value 1 Joint download boot mode 3 1 1 0" (原文明确说明)
- **C-C3C-RESET** CHIP_EN low powers the chip off; it must not be left floating — ESP32-C3 的复位脚 7: 有效电平 low
  - 依据: SRC-ESP32C3-DS: ESP32-C3 Series Datasheet v2.5 [官方手册], 第 25 页, Table 2-1. Pin Overview — "Do not leave the CHIP_EN pin floating" (原文明确说明)
  - 依据: SRC-ESP32C3-DS: ESP32-C3 Series Datasheet v2.5 [官方手册], 第 25 页, Table 2-1. Pin Overview — "Do not leave the CHIP_EN pin floating" (原文明确说明)
- **C-C3C-ELECTRICAL-SUPPLY** VDDA, VDD3P3 and VDD3P3_RTC take 3.0 V to 3.6 V (3.3 V typical); the cumulative input current is at least 0.5 A — VDDA, VDD3P3, VDD3P3_RTC input voltage and cumulative input current (Table 5-2. Recommended Operating Conditions): 最小值 3.0 V; 典型值 3.3 V; 最大值 3.6 V; min current 0.5 A
  - 依据: SRC-ESP32C3-DS: ESP32-C3 Series Datasheet v2.5 [官方手册], 第 55 页, Table 5-2. Recommended Operating Conditions — "VDDA, VDD3P3, VDD3P3_RTC Recommended input voltage 3.0 3.3 3.6 V" (原文明确说明)
  - 依据: SRC-ESP32C3-DS: ESP32-C3 Series Datasheet v2.5 [官方手册], 第 55 页, Table 5-2. Recommended Operating Conditions — "IV DD Cumulative input current 0.5" (原文明确说明)

## 2. 必需外设与必须保留项

- **BLK-C3C-CRYSTAL** External 40 MHz crystal (compulsory) (generic)
  - crystal: 1 × other
  - 依据: SRC-ESP32C3-HDG-SCH: ESP32-C3 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], Clock Source — "External crystal clock source (Compulsory)" (原文明确说明)
  - 依据: SRC-ESP32C3-HDG-SCH: ESP32-C3 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], External Crystal Clock Source (Compulsory) — "The ESP32-C3 firmware only supports 40 MHz crystal." (原文明确说明)

## 3. 可配置项、可选项与推导项

[可配置]
- **C-C3C-NEED-RESET** An RC delay circuit on CHIP_EN is advised: usually 10 kΩ and 1 µF, adjusted to the supply — 必须有外设块 RC delay circuit on CHIP_EN (usually 10 kΩ and 1 µF)
  - 严重级别: 警告（不使校验失败）
  - 依据: SRC-ESP32C3-HDG-SCH: ESP32-C3 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], Chip Power-up and Reset Timing — "it is advised to add an RC delay circuit at the CHIP_EN pin." (原文明确说明)
  - 依据: SRC-ESP32C3-HDG-SCH: ESP32-C3 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], Chip Power-up and Reset Timing — "The recommended setting for the RC delay circuit is usually R = 10 kΩ and C = 1 μF." (原文明确说明)
[推导]
- **C-C3C-NEED-BOOT** Download mode needs GPIO9 pulled low (and GPIO8 high): a button, a jumper or a programmer's output — 必须有外设块 A way to pull GPIO9 low (button or jumper) to enter download mode
  - 推导自: C-C3C-BOOT-GPIO9 (这是推导，不是手册原文)
  - 严重级别: 警告（不使校验失败）
  - 依据: SRC-ESP32C3-DS: ESP32-C3 Series Datasheet v2.5 [官方手册], 第 31 页, Table 3-3. Chip Boot Mode Control — "SPI boot mode 1 Any value 1 Joint download boot mode 3 1 1 0" (由多条规则推导)
- **BLK-C3C-RESET** RC delay circuit on CHIP_EN (usually 10 kΩ and 1 µF) (reset_circuit)
  - pull_up: 1 × resistor (数值 10000 Ω)
  - delay: 1 × capacitor (最小值 1000 nF)
  - 连接 7 → *
  - 依据: SRC-ESP32C3-HDG-SCH: ESP32-C3 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], Chip Power-up and Reset Timing — "The recommended setting for the RC delay circuit is usually R = 10 kΩ and C = 1 μF." (原文明确说明)
- **BLK-C3C-BOOT** A way to pull GPIO9 low (button or jumper) to enter download mode (boot_button)
  - button: 1 × switch
  - 连接 15 → *
  - 依据: SRC-ESP32C3-DS: ESP32-C3 Series Datasheet v2.5 [官方手册], 第 31 页, Table 3-3. Chip Boot Mode Control — "SPI boot mode 1 Any value 1 Joint download boot mode 3 1 1 0" (由多条规则推导)

## 4. AI 可编辑区域（白名单）

默认所有原理图、PCB、引脚和机械对象只读；只有下面列出的内容可以被 AI 修改，其余一律不得改动。

（没有 base_design：没有任何东西可由 AI 修改）

## 5. 高速 / 机械 / 布线约束

[configurable]
- **C-C3C-USB** USB differential pair: 90 Ω ±10%, in parallel and of equal length — 接口网络 USB_D+, USB_D-, USB_DP, USB_DM (差分) (阻抗 90 Ω; impedance tolerance 10 %)
  - 依据: SRC-ESP32C3-HDG-PCB: ESP32-C3 Hardware Design Guidelines - PCB Layout Design latest-2026-11-16 [官方设计指南], USB — "Use differential pairs and route them in parallel at equal lengths. Maintain a differential pair impedance of 90 Ω with a tolerance of ±10%." (原文明确说明)

## 6. 冲突与缺口

- **C-C3C-USB-SKEW** 资料未说明: 接口网络 USB_D+, USB_D-, USB_DP, USB_DM (差分) (偏斜 未知)
  - 缺口: The guideline says `route them in parallel at equal lengths` and gives no number; no document read for this pack gives a skew limit in millimetres: no typical value is used.
- **C-C3C-EN-RC-VALUES** 资料未说明: 必须有外设块 RC delay circuit on CHIP_EN (usually 10 kΩ and 1 µF)
  - 缺口: The guideline says `usually R = 10 kΩ and C = 1 μF` and that the values are adjusted to the power supply and the reset timing of the project. No document gives final values.

## 7. 来源列表

| 编号 | 文档 | 版本 | 级别 | 获取日期 | SHA-256 |
|---|---|---|---|---|---|
| SRC-ESP32C3-DS / DOC-ESP32C3-DS | ESP32-C3 Series Datasheet (https://www.espressif.com/sites/default/files/documentation/esp32-c3_datasheet_en.pdf) | v2.5 | 官方手册 | 2026-11-16 | `fede706c39ce663c` |
| SRC-ESP32C3-HDG-SCH / DOC-ESP32C3-HDG-SCH | ESP32-C3 Hardware Design Guidelines - Schematic Checklist (https://docs.espressif.com/projects/esp-hardware-design-guidelines/en/latest/esp32c3/schematic-checklist.html) | latest-2026-11-16 | 官方设计指南 | 2026-11-16 | `43cd800e9ab35b5c` |
| SRC-ESP32C3-HDG-PCB / DOC-ESP32C3-HDG-PCB | ESP32-C3 Hardware Design Guidelines - PCB Layout Design (https://docs.espressif.com/projects/esp-hardware-design-guidelines/en/latest/esp32c3/pcb-layout-design.html) | latest-2026-11-16 | 官方设计指南 | 2026-11-16 | `1dc118cbe00ee1fc` |

回跳原文：`hcp-manager evidence verify <pack>` 检查每条引用的文档、版本、摘要和页码上的原文；`hcp-manager search "…" --device <型号>` 找到同一版本的原文块。
