# ESP32-S3-WROOM-1-N8R8: example hardware constraint pack

A Hardware Constraint Pack for one module, written from the official Espressif documents that were locked on 2026-11-16, with the
checks, reports and tests that go with it. Read `acceptance.md` for what works, what does not and what a person still has to confirm.

**Locked device**: ESP32-S3-WROOM-1-N8R8 (8 MB quad flash, 8 MB octal PSRAM). The variant matters: IO35, IO36 and IO37 are reserved
because of the octal PSRAM, and the other variants of the same datasheet differ.

| File | What it is |
|---|---|
| `workspace/sources.yaml` | the inventory of documents, managed by `hcp-manager`: URL, level, version, SHA-256 of what was read |
| `overlay.template.yaml` → `pack.yaml` | the example's own part (the carrier: outline, mounting holes, the free area, the LED block, the fixed base design). `pack.yaml` is **generated**: this overlay merged with the module pack `espressif/esp32-s3/esp32-s3-wroom-1/n8r8` (its facts: 19 rules, 41 pins, 4 blocks, the antenna keep-out), which itself inherits from the chip pack `espressif/esp32-s3/esp32-s3/qfn56` (`hardware-packs/`). There is one copy of the module's facts, not two; the merged pack has the same 23 rules, 41 pins, 5 blocks and 6 regions as before (compared object by object when this was done) |
| `ir.json` | the compiled Constraint IR |
| `reports/pack-report.zh.md` | the Chinese report: fixed / required / configurable / unknown, with the quoted evidence |
| `reports/validation-legal.{json,zh.md}` | the validation of the legal carrier board |
| `example/` | the made-up carrier board (`carrier.kicad_pcb`, KiCad 8) and its notes, the `open_board` source |
| `eval/retrieval-eval.yaml` | what a search must find, and what it must never return |
| `tools/` | `extract_pin_table.py` (pin table from the datasheet; `--check pack.yaml`), `build_pack.py` (calls `hardware-packs/tools/build_espressif.py`), `regenerate.sh` |

The documents are not in the repository (they are Espressif's). The inventory pins their digests; `hcp-manager` downloads them and
refuses a file that does not match.

## Reproduce

```bash
cargo build --release -p fastroute -p hcp-manager
export PATH="$PWD/target/release:$PATH"
cd hardware-packs/espressif/esp32-s3-wroom-1-n8r8

hcp-manager --workspace workspace sources sync          # download, check the pinned digests
hcp-manager --workspace workspace index build           # derived text blocks (rebuildable)
hcp-manager --workspace workspace search --device ESP32-S3-WROOM-1-N8R8 "Do not leave the EN pin floating"
hcp-manager --workspace workspace evidence verify pack.yaml    # every quotation is on the cited page

fastroute constraints check pack.yaml
fastroute constraints validate --pack pack.yaml --kicad-pcb example/carrier.kicad_pcb --report validation.md   # exit 5: gaps are named
fastroute constraints edit-request --pack pack.yaml --requirement "add a status LED" -o request.json
tools/regenerate.sh                                     # rebuild pack.yaml, ir.json and the reports
```

The whole flow in one run, through the Skill's script:

```bash
python3 ../../../skills/hardware-constraint-design/scripts/hcp_cli.py --workspace workspace \
    pipeline pack.yaml --out /tmp/esp-out --kicad-pcb example/carrier.kicad_pcb
```

Tests: `cargo test -p fastroute --test esp32s3_pack --test esp32s3_documents` (the second one needs `hcp-manager` built and either the
cache or a network; it says so and passes otherwise).
