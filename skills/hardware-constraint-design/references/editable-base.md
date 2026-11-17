# A fixed base design and the regions an AI may edit

Read this when the job is "keep this open board / core module exactly as it is and add something around
it". The AI does not redraw the board. It hands over a **draft**: a list of changes, each inside a region
that the pack authorises. A deterministic checker accepts or rejects the draft and prints what would change.
Nothing here uses a network, a model or a vector index.

## The rule

Everything is read-only. Only what the pack lists under `base_design.editable_regions` may change, and only
with the operations listed there. A search result — keyword or vector — can point at a block or a document;
it never adds a region, a block, a pin or an operation to the list.

## In the pack

```yaml
base_design:
  id: BASE-FM100-CARRIER
  binding:                      # what the pack was made for: a commit and/or file digests
    repo: { url: https://example.invalid/open-board, commit: "<40 hex>" }
    files:
      - { path: carrier.kicad_pcb, sha256: "<64 hex>" }
  immutable:                    # named so that nobody has to guess
    components: [U1, J1]
    connectors: [J1]
    board_outline: true
    mount_holes: true
    regions: [REG-ANT]          # keep-outs and other regions that stay as they are
  editable_regions:
    - region: REG-USER          # a region of kind `editable`, modifiability `ai_editable`
      allowed_blocks: [BLK-LED] # peripheral blocks (modifiability ai_editable) that may be added here
      pin_pool:                 # pins of the device that may be used here
        - { device: DEV-FM100, pins: ["5"] }
      allowed_ops: [add_component, connect, add_track]   # no list, no operation
```

- A pin that a rule protects (reserved, no-connect, strapping, reset, boot, fixed net, or a power / ground /
  RF pin in the pin table) cannot be in a pool: `hcp check` says `pool_pin_protected`, and the checker leaves
  it out of the pool anyway.
- Identity of base objects: the reference designator, plus the `uuid` of the component in the board when the
  draft gives one (KiCad's footprint id). A draft that means another uuid is `stale_reference`: the board was
  renumbered.
- `binding.files[].sha256` is the SHA-256 of the file in the open board's repository at the commit the pack
  was made for. The pack, the base and the draft refer to each other by digest; nothing refers to "latest".

## The request

```bash
fastroute constraints edit-request --pack pack.yaml --requirement "add a status LED" -o request.json
```

The request (`schemas/edit-request-v1.schema.json`) holds the pack (id, version, sha256), the base (id,
binding, binding digest), the requirement, and the whitelist as computed from the pack: regions with their
polygons and layers, the allowed blocks (with their components and evidence), the pin pools without protected
pins, the allowed operations, the immutable objects, the fixed regions, and the protected pins with the rule
that protects each. The same pack gives the same bytes. The knowledge pack and the base design are managed by
`hcp-manager` (locked sources, digests); a request names exactly one version of each.

## The draft

`schemas/edit-draft-v1.schema.json`; an example is `assets/example-edit-draft.json`. A draft has the pack
and base digests copied from the request, optional `claims` (what the generator thinks it may use — checked,
and refused when it asks for more), optional `recall` (what a search found — information only), an optional
`stop` (the generator could not finish: reason, what is missing — no changes may come with it), and the
`changes`. Each change names its `region`:

| op | fields | notes |
| --- | --- | --- |
| `add_component` | `block`, `role`, `ref`, `value?`, `footprint?`, `at` | `block` must be allowed in the region, `role` must exist in the block |
| `move_component` | `ref`, `uuid?`, `to` | base parts only if they are inside the region, are not immutable, and are not devices |
| `remove_component` | `ref`, `uuid?` | same |
| `connect` | `ref`, `pin`, `net`, `uuid?` | pins of added parts, of base parts inside the region, or a pin of the pool |
| `disconnect` | `ref`, `pin`, `uuid?` | never a pin of a device |
| `add_track` | `net`, `layer`, `width_mm`, `points` | the whole copper (with its width) inside the region, outside fixed regions |
| `add_via` | `net`, `at`, `diameter_mm` | same |

A draft is never applied to the project. The checker applies it to an in-memory copy to run the pack's rules.
Writing the accepted draft into the project is a separate step that a person takes with their own tools.

## The check

```bash
fastroute constraints check-edit draft.json --pack pack.yaml \
    --kicad-pcb carrier.kicad_pcb --base-dir ./carrier-repo --report diff.md
```

| exit | meaning |
| --- | --- |
| 0 | accepted, and everything was checked (base digests compared, rules run) |
| 5 | accepted with parts not checked; or the pack / the base changed (review); or the generator stopped |
| 1 | rejected: a change is outside what is authorised, or breaks a rule of the pack |
| 3 | not a draft (schema), a missing file |
| 4 | an output would overwrite an input, a bound file, or anything in `--base-dir` |

Order of judgement: (1) is it the draft for this pack and this base — `pack_mismatch`, `base_mismatch`,
`base_changed` mean **review**, not accept; (2) does the draft claim more than the whitelist; (3) every change
against the whitelist and the base; (4) the pack's rules on the result, before and after: only *new*
failures count against the draft.

Findings (JSON `findings[].code`):

| code | meaning |
| --- | --- |
| `pack_mismatch` | the draft is for another version of the pack: review |
| `base_mismatch` | the draft is bound to another base binding: review |
| `base_changed` | a bound file differs from the working copy (or the design) — constraints may have drifted: review |
| `base_unverified` | warning: the base files were not compared (give `--base-dir`, or a board that is a bound file) |
| `widened_authorization` | `claims` asks for a region, block or pin that the pack does not list |
| `no_base_design` | the pack has no `base_design`: nothing is editable |
| `region_not_editable`, `op_not_allowed` | the region is not on the list, or does not allow the operation |
| `unauthorized_block`, `unknown_role` | the block is not allowed in the region (a search result does not change that), or has no such role |
| `outside_region`, `outside_board`, `crosses_region`, `enters_keepout`, `layer_not_allowed` | geometry: outside the region, the board, over a border (cross-region routing), into a fixed region (with its evidence), on a layer the region does not cover |
| `immutable_object`, `base_object_modified`, `ref_exists`, `duplicate_ref`, `ref_unknown`, `stale_reference` | fixed or unknown objects, replaced references, changed identities |
| `reserved_pin_used`, `pin_not_in_pool`, `pin_in_use`, `protected_net` | pins and nets that may not be used (with the rule and its evidence) |
| `base_required` | warning-level need: a change to a base part cannot be judged without the design |
| `rule_violation` | after the draft, a rule of the pack fails that did not fail before (with the rule's evidence) |
| `stopped`, `stop_with_changes`, `empty_draft` | the draft stops / stops and still has changes / has nothing |
| `recall_not_authorized` | warning: a block that a search found is not on the whitelist; it is ignored |
| `validator_not_run` | warning: no design was given, so only the whitelist was checked |

## The report

`--report diff.md` (Chinese by default, `--lang en`): the verdict; what the draft is bound to; every change with
what authorises it (`base_design.editable_regions[REG-USER].allowed_blocks: BLK-LED`) or the code that refused
it, with the evidence of the block or of the rule that was broken; the counts before and after (components,
connections, copper); the pack's rules before and after, with the failures the draft adds and the ones it
removes; and what the check does not cover (clearance and shorts against the rest of the board are KiCad's
DRC's job).

## When to stop instead of drafting

Write `stop` with a reason and the missing information when: the requirement needs a block, region, pin or
operation that is not on the list; a document that the block needs is missing; two rules conflict; or the
base design changed. Do not widen the authority to make the requirement fit.
