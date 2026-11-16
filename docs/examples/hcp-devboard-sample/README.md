# Sample board pack (fictional)

`board-pack.yaml` is a `kind: board` Hardware Constraint Pack for a **made-up** 20 mm x 60 mm development board, and `request.json` is a request to embed it in a
100 mm x 100 mm carrier. It exists to show the shape of a board pack and to be run by anyone, offline, without a licence question: both files are released
under CC0 1.0 (`LICENSE.txt`). It is **not** a real product; its quotations are invented and its source URL is `example.invalid`.

It is not a real board. A pack for a real development board (for example Espressif's ESP32-S3-DevKitC-1) has to be written from that board's documents
and checked against them; that has not been done, and the redistribution terms of the vendor's schematics and drawings have to be read before they are
quoted or shipped. See [../../hcp-board-embedding.md](../../hcp-board-embedding.md) for what a board pack must say.

```bash
cargo build --release -p hcp-manager
WS=$(mktemp -d)
mkdir -p "$WS/packs/acme/devs/devboard/rev-b/0.1.0"
cp board-pack.yaml "$WS/packs/acme/devs/devboard/rev-b/0.1.0/pack.yaml"
hcp-manager --workspace "$WS" packs embed --board acme/devs/devboard/rev-b --request request.json --out out/ --lang en
```

The result is `out/`: `embedding.json`, `carrier-pack.yaml`, `ir.json`, `diff-report.json`, `report.en.md`, `board-embedding-request.json`. The files are kept equal
to what `apps/hcp-manager/tests/board.rs` builds (`the_sample_board_pack_is_what_the_tests_build_and_it_embeds_offline`); regenerate them with
`HCP_WRITE_SAMPLE=1 cargo test -p hcp-manager --test board sample`.
