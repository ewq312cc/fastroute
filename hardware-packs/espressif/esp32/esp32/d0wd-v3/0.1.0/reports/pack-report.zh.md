# ESP32 (ESP32-D0WD-V3, QFN48): chip-level constraints — 硬件约束知识包报告

> 这份报告回答四个问题：**什么不能动、什么必须保留、什么可以改、依据是什么**。每一条都列出来源文档、版本、页码/章节和原文摘录；“推导”“参考板观察”“待确认”不会被写成手册原文。

- 知识包: `espressif/esp32/esp32/d0wd-v3` 0.1.0 · 抽取者: mixed
- 审查状态: **draft**（草稿：尚未经人工审查，不得作为最终依据）
- 器件: Espressif ESP32-D0WD-V3  (DEV-ESP32-D0WD-V3), 9 个引脚

## 1. 不可修改项（固定）

- **C-ESP32C-STRAP-GPIO0** GPIO0 is a strapping pin: its default level at reset is high — ESP32-D0WD-V3 的启动配置脚 23: 采样时刻 reset, 默认电平 high, 允许外部驱动
  - 依据: SRC-ESP32-DS: ESP32 Series Datasheet v5.3 [官方手册], 第 22 页, Table 3-1. Default Configuration of Strapping Pins — "GPIO0 Pull-up 1" (原文明确说明)
- **C-ESP32C-STRAP-GPIO2** GPIO2 is a strapping pin: its default level at reset is low — ESP32-D0WD-V3 的启动配置脚 22: 采样时刻 reset, 默认电平 low, 允许外部驱动
  - 依据: SRC-ESP32-DS: ESP32 Series Datasheet v5.3 [官方手册], 第 22 页, Table 3-1. Default Configuration of Strapping Pins — "GPIO2 Pull-down 0" (原文明确说明)
- **C-ESP32C-STRAP-MTDI** MTDI is a strapping pin: its default level at reset is low — ESP32-D0WD-V3 的启动配置脚 18: 采样时刻 reset, 默认电平 low, 允许外部驱动
  - 依据: SRC-ESP32-DS: ESP32 Series Datasheet v5.3 [官方手册], 第 22 页, Table 3-1. Default Configuration of Strapping Pins — "MTDI Pull-down 0" (原文明确说明)
- **C-ESP32C-STRAP-MTDO** MTDO is a strapping pin: its default level at reset is high — ESP32-D0WD-V3 的启动配置脚 21: 采样时刻 reset, 默认电平 high, 允许外部驱动
  - 依据: SRC-ESP32-DS: ESP32 Series Datasheet v5.3 [官方手册], 第 22 页, Table 3-1. Default Configuration of Strapping Pins — "MTDO Pull-up 1" (原文明确说明)
- **C-ESP32C-STRAP-GPIO5** GPIO5 is a strapping pin: its default level at reset is high — ESP32-D0WD-V3 的启动配置脚 34: 采样时刻 reset, 默认电平 high, 允许外部驱动
  - 依据: SRC-ESP32-DS: ESP32 Series Datasheet v5.3 [官方手册], 第 22 页, Table 3-1. Default Configuration of Strapping Pins — "GPIO5 Pull-up 1" (原文明确说明)
- **C-ESP32C-BOOT-GPIO0** GPIO0 and GPIO2 both low select the Joint Download Boot mode — ESP32-D0WD-V3 的启动模式脚 23: 电平 low → Joint Download Boot (SDIO or UART download); GPIO2 must be low as well
  - 依据: SRC-ESP32-DS: ESP32 Series Datasheet v5.3 [官方手册], 第 23 页, 3.1 Chip Boot Mode Control — "GPIO0 and GPIO2 control the boot mode after the reset is released." (原文明确说明)
  - 依据: SRC-ESP32-DS: ESP32 Series Datasheet v5.3 [官方手册], 第 23 页, Table 3-3. Chip Boot Mode Control — "SPI Boot Mode 1 Any value Joint Download Boot Mode 2 0 0" (原文明确说明)
- **C-ESP32C-RESET** CHIP_PU low powers the chip off; it must not be left floating — ESP32-D0WD-V3 的复位脚 9: 有效电平 low
  - 依据: SRC-ESP32-DS: ESP32 Series Datasheet v5.3 [官方手册], 第 14 页, Table 2-1. Pin Overview — "Low: Off; the chip shuts down" (原文明确说明)
  - 依据: SRC-ESP32-DS: ESP32 Series Datasheet v5.3 [官方手册], 第 14 页, Table 2-1. Pin Overview — "Note: Do not leave the CHIP_PU pin floating." (原文明确说明)
- **C-ESP32C-ELECTRICAL-SUPPLY** Analog and RTC supply pins take 2.3 V to 3.6 V — VDDA, VDD3P3 and VDD3P3_RTC supply voltage (as stated for the pins in Table 2-1 Pin Overview): 最小值 2.3 V; 最大值 3.6 V
  - 依据: SRC-ESP32-DS: ESP32 Series Datasheet v5.3 [官方手册], 第 14 页, Table 2-1. Pin Overview — "VDDA 1 P Analog power supply (2.3 V ∼ 3.6 V)" (原文明确说明)
  - 依据: SRC-ESP32-DS: ESP32 Series Datasheet v5.3 [官方手册], 第 14 页, Table 2-1. Pin Overview — "VDD3P3_RTC 19 P Input power supply for RTC IO (2.3 V ∼ 3.6 V)" (原文明确说明)

## 2. 必需外设与必须保留项

- **BLK-ESP32C-CRYSTAL** External 40 MHz crystal (compulsory) (generic)
  - crystal: 1 × other
  - 依据: SRC-ESP32-HDG-SCH: ESP32 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], Clock Source — "External crystal clock source (Compulsory)" (原文明确说明)
  - 依据: SRC-ESP32-HDG-SCH: ESP32 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], External Crystal Clock Source (Compulsory) — "The ESP32 firmware only supports 40 MHz crystal." (原文明确说明)

## 3. 可配置项、可选项与推导项

[可配置]
- **C-ESP32C-NEED-RESET** An RC delay circuit on CHIP_PU is advised: usually 10 kΩ and 1 µF, adjusted to the supply — 必须有外设块 RC delay circuit on CHIP_PU (usually 10 kΩ and 1 µF)
  - 严重级别: 警告（不使校验失败）
  - 依据: SRC-ESP32-HDG-SCH: ESP32 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], Chip Power-up and Reset Timing — "it is advised to add an RC delay circuit at the CHIP_PU pin." (原文明确说明)
  - 依据: SRC-ESP32-HDG-SCH: ESP32 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], Chip Power-up and Reset Timing — "The recommended setting for the RC delay circuit is usually R = 10 kΩ and C = 1 μF." (原文明确说明)
[推导]
- **C-ESP32C-NEED-BOOT** Download mode needs GPIO0 pulled low: a button, a jumper or a programmer's output — 必须有外设块 A way to pull GPIO0 low (button or jumper) to enter download mode
  - 推导自: C-ESP32C-BOOT-GPIO0 (这是推导，不是手册原文)
  - 严重级别: 警告（不使校验失败）
  - 依据: SRC-ESP32-DS: ESP32 Series Datasheet v5.3 [官方手册], 第 23 页, Table 3-3. Chip Boot Mode Control — "SPI Boot Mode 1 Any value Joint Download Boot Mode 2 0 0" (由多条规则推导)
- **BLK-ESP32C-RESET** RC delay circuit on CHIP_PU (usually 10 kΩ and 1 µF) (reset_circuit)
  - pull_up: 1 × resistor (数值 10000 Ω)
  - delay: 1 × capacitor (最小值 1000 nF)
  - 连接 9 → *
  - 依据: SRC-ESP32-HDG-SCH: ESP32 Hardware Design Guidelines - Schematic Checklist latest-2026-11-16 [官方设计指南], Chip Power-up and Reset Timing — "The recommended setting for the RC delay circuit is usually R = 10 kΩ and C = 1 μF." (原文明确说明)
- **BLK-ESP32C-BOOT** A way to pull GPIO0 low (button or jumper) to enter download mode (boot_button)
  - button: 1 × switch
  - 连接 23 → *
  - 依据: SRC-ESP32-DS: ESP32 Series Datasheet v5.3 [官方手册], 第 23 页, Table 3-3. Chip Boot Mode Control — "SPI Boot Mode 1 Any value Joint Download Boot Mode 2 0 0" (由多条规则推导)

## 4. AI 可编辑区域（白名单）

默认所有原理图、PCB、引脚和机械对象只读；只有下面列出的内容可以被 AI 修改，其余一律不得改动。

（没有 base_design：没有任何东西可由 AI 修改）

## 5. 高速 / 机械 / 布线约束

（没有）

## 6. 冲突与缺口

- **C-ESP32C-EN-RC-VALUES** 资料未说明: 必须有外设块 RC delay circuit on CHIP_PU (usually 10 kΩ and 1 µF)
  - 缺口: The guideline says `usually R = 10 kΩ and C = 1 μF` and that the values have to be adjusted to the power supply and the power-up and reset timing of the project. No document gives the final values: the project decides and tests them.

## 7. 来源列表

| 编号 | 文档 | 版本 | 级别 | 获取日期 | SHA-256 |
|---|---|---|---|---|---|
| SRC-ESP32-DS / DOC-ESP32-DS | ESP32 Series Datasheet (https://www.espressif.com/sites/default/files/documentation/esp32_datasheet_en.pdf) | v5.3 | 官方手册 | 2026-11-16 | `a7917e6b47528c9d` |
| SRC-ESP32-HDG-SCH / DOC-ESP32-HDG-SCH | ESP32 Hardware Design Guidelines - Schematic Checklist (https://docs.espressif.com/projects/esp-hardware-design-guidelines/en/latest/esp32/schematic-checklist.html) | latest-2026-11-16 | 官方设计指南 | 2026-11-16 | `db4527861d48b048` |

回跳原文：`hcp-manager evidence verify <pack>` 检查每条引用的文档、版本、摘要和页码上的原文；`hcp-manager search "…" --device <型号>` 找到同一版本的原文块。
