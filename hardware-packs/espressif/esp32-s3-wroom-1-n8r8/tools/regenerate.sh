#!/bin/sh
# Rebuilds everything that is derived from pack.template.yaml and the example carrier, with the programs of this repository:
# pack.yaml, the compiled Constraint IR, the Chinese pack report and the validation report of the legal carrier.
# Needs the cached datasheet (hcp-manager --workspace workspace sources sync) for the pin table; run from any directory.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
fr=${FASTROUTE_BIN:-fastroute}
cd "$here"
python3 example/make_carrier.py
python3 tools/build_pack.py
"$fr" constraints check pack.yaml
"$fr" constraints compile pack.yaml -o ir.json --force -q
"$fr" constraints report pack.yaml -o reports/pack-report.zh.md --lang zh --force -q
# 5 = nothing violated, but unknown / unsupported / unchecked rules remain: that is the expected result for the legal example
"$fr" constraints validate --ir ir.json --kicad-pcb example/carrier.kicad_pcb -o reports/validation-legal.json --report reports/validation-legal.zh.md --lang zh --force -q || [ $? -eq 5 ]
