# 硬件约束校验报告

- 结果: **通过**（不完整：有未检查、不支持、未知或冲突的规则）
- 知识包: espressif/esp32-s3-wroom-1-n8r8 0.1.0 (pack sha256 `837cf0feddf66c25f50bf78cd616fd926306b633bd4d0390e749fbe3786e8753`)
- Constraint IR sha256 `e3e34d20d2e007c53260b800d6484061663bfd67bc6f160479c5177a9843f038`
- 设计: carrier (sha256 `fc9f0fb32489a6c1502c11ef428e34e9b4f6c7666150bb99b91faaf3ff749c90`)

## 摘要

| 状态 | 规则数 |
|---|---|
| 通过 | 18 |
| 失败 | 0 |
| 不适用 | 0 |
| 未检查 | 2 |
| 不支持 | 1 |
| 资料未说明 | 3 |
| 资料冲突 | 0 |

## 未检查 (2)

### C-ESP-USB `high_speed`

- 说明: the parameter 'impedance_ohm' is not checked (error)
- 依据: SRC-ESP32S3-HDG-PCB USB (explicit): "Use differential pairs and route them in parallel at equal lengths. Maintain a differential pair impedance of 90 Ω with a tolerance of ±10%."
- 依据: SRC-ESP32S3-HDG-SCH USB (explicit): "GPIO19 and GPIO20 can be used as D- and D+ of USB respectively."

### PINMUX-ESP32S3-WROOM1-N8R8 `pin_functions` — U1

- 说明: the design has no pin assignments for U1: pin functions cannot be checked (error)
- 依据: SRC-ESP32S3-WROOM1-DS p.11, Table 3-1. Pin Definitions (explicit): "GND 1 P GND"
- 依据: SRC-ESP32S3-WROOM1-DS p.11, Table 3-1. Pin Definitions (explicit): "3V3 2 P Power supply"
- 依据: SRC-ESP32S3-WROOM1-DS p.11, Table 3-1. Pin Definitions (explicit): "EN 3 I"
- 依据: SRC-ESP32S3-WROOM1-DS p.11, Table 3-1. Pin Definitions (explicit): "IO4 4 I/O/T RTC_GPIO4, GPIO4, TOUCH4, ADC1_CH3"
- 依据: SRC-ESP32S3-WROOM1-DS p.11, Table 3-1. Pin Definitions (explicit): "IO5 5 I/O/T RTC_GPIO5, GPIO5, TOUCH5, ADC1_CH4"
- 另有 36 条依据，见 JSON 结果

## 不支持 (1)

### C-ESP-ELECTRICAL-VDD33 `electrical`

- 说明: validator: there is no electrical model; a limit of the device is reported, not checked (error)
- 依据: SRC-ESP32S3-WROOM1-DS p.27, Table 6-2. Recommended Operating Conditions (explicit): "VDD33 Power supply voltage 3.0 3.3 3.6 V"
- 依据: SRC-ESP32S3-WROOM1-DS p.27, Table 6-2. Recommended Operating Conditions (explicit): "Current delivered by external power supply 0.5"

## 资料未说明 (3)

### C-ESP-EN-RC-VALUES `required_block`

- 说明: the documents do not say: Figure 9-1 of the module datasheet v1.8 (page 41) shows R1 and C2 of the EN circuit as "TBD". The text of the same page and the Hardware Design Guidelines say "usually R = 10 kΩ and C = 1 µF" and tell the designer to adjust the values to the power-up timing of the module and the chip (rule C-ESP-NEED-RESET keeps the usual values as a recommendation). The final values depend on the supply ramp of the project, which no document gives: the project has to decide them and test the power-up. (error)

### C-ESP-USB-GAP `high_speed`

- 说明: the documents do not say: The gap between the two traces depends on the stack-up and the trace width; the documents read give the 90 Ω target but no spacing. The stack-up of the board is needed (the plugin's impedance tool can derive widths from it). (error)

### C-ESP-USB-SKEW `high_speed`

- 说明: the documents do not say: The PCB Layout Design guideline says to route the pair "in parallel at equal lengths" and gives no number. No document read for this pack gives a skew limit in millimetres; do not use a typical value. (error)

## 通过 (19)

- C-ESP-ANT-KEEPOUT `keepout`
- C-ESP-BOOT-IO0 `boot_pin`
- C-ESP-EN `reset_pin`
- C-ESP-GND-1 `fixed_net`
- C-ESP-GND-40 `fixed_net`
- C-ESP-MOUNT-H1 `mechanical_fixed`
- C-ESP-MOUNT-H2 `mechanical_fixed`
- C-ESP-MOUNT-H3 `mechanical_fixed`
- C-ESP-NEED-BOOT `required_block`
- C-ESP-NEED-POWER `required_block`
- C-ESP-NEED-RESET `required_block`
- C-ESP-NEED-UART `required_block`
- C-ESP-OUTLINE `mechanical_fixed`
- C-ESP-RESERVED-PSRAM `reserved_pin`
- C-ESP-STRAP-IO0 `strapping`
- C-ESP-STRAP-IO3 `strapping`
- C-ESP-STRAP-IO45 `strapping`
- C-ESP-STRAP-IO46 `strapping`
- C-ESP-USB `high_speed`

