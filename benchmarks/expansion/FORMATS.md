# Non-KiCad intake and Konnect evaluation

Evaluated 2026-10-18 on this host: KiCad CLI and system Python `pcbnew` are
**10.0.7**. No additional software was installed. Native KiCad intake should
remain the high-throughput path; non-KiCad originals and conversion reports
are kept separately so conversion defects do not become routing labels.

## Konnect decision

[Konnect](https://github.com/mixelpixx/Konnect) is useful for interactive,
agent-assisted editing through KiCad 10's IPC API. Most PCB operations need
the editor running with the target board open; its checks and exports use
`kicad-cli`. Its limited file-based placement fallback does not provide a
batch acquisition or format-validation pipeline. Its Specctra workflow also
routes through Freerouting, whereas this collection must measure pcb-maker.

**Do not add it to the unattended harvest.** The installed CLI already supplies
import, statistics and DRC without a GUI, MCP round trips or LLM decisions.
Consider Konnect later for inspecting individual quarantined conversions or
interactive placement repair. This is a fit assessment from its published
README/tool descriptions, not a claim that Konnect was installed or tested.

## Installed headless format support

The [KiCad 10 CLI documentation](https://docs.kicad.org/10.0/en/cli/cli.html#_pcb_import)
and local `kicad-cli pcb import --help` agree on these format hints:

| Source | Hint | Intake decision |
|---|---|---|
| Eagle XML `.brd` | `eagle` | Valuable production-board source; tested importer defect below requires quarantine |
| Altium `.PcbDoc` | `altium` | Tested successful native read-back; preserve original rules and validate semantics before labels |
| PADS | `pads` | CLI supported; not smoke-tested here |
| CADSTAR | `cadstar` | CLI supported; not smoke-tested here |
| Fabmaster | `fabmaster` | CLI supported; not smoke-tested here |
| P-CAD | `pcad` | CLI supported; not smoke-tested here |
| SolidWorks PCB | `solidworks` | CLI supported; not smoke-tested here |

Layers are automatically mapped by KiCad. Explicit hints prevent ambiguous
`.brd` files from selecting the wrong importer. Binary pre-XML Eagle files
are not accepted by this wrapper's Eagle inventory. EasyEDA import plugins
exist in `pcbnew`, but the installed CLI does not expose a matching explicit
hint, so they are not an established unattended path here. Gerbers are useful
for geometric comparison; they are insufficient by themselves as placement
and routing labels because component identities, pad connectivity and design
rules are missing.

The wrapper records CLI version, source SHA-256, source Eagle inventory when
applicable, importer messages, layer mapping, native statistics and refilled
DRC baseline. Every CLI invocation has a wall timeout and its process group
is killed on expiry. On this Linux host `prlimit` bounds address space to
8 GiB and disables core dumps. Single-thread environment settings are requested;
native KiCad checks can still create helper threads, so the pipeline also limits
concurrent checks.

```sh
python3 benchmarks/expansion/convert.py source.PcbDoc output-directory \
  --format altium --timeout 60 --metadata provenance.json
```

Equivalent native operations:

```sh
kicad-cli pcb import --format altium --output converted.kicad_pcb \
  --report-format json --report-file import.json source.PcbDoc
kicad-cli pcb export stats --format json --output stats.json converted.kicad_pcb
kicad-cli pcb drc --format json --severity-all --refill-zones \
  --output drc.json converted.kicad_pcb
```

This DRC command deliberately does not save the refilled board: the first
conversion remains available unchanged for comparing geometry. A separate
project-rule file is not fabricated. Without source-equivalent project
rules, this baseline can contain many default-rule violations.

## Real-source smoke tests

Files are local under `data/non-kicad/`, excluded from git. Each directory
contains `source.*`, `provenance.json`, converted output and `conversion.json`.
The original licence/attribution is retained locally.

| Source | Pinned commit | Imported geometry | Gate result |
|---|---|---|---|
| [Adafruit Feather ESP32-S3](https://github.com/adafruit/Adafruit-Feather-ESP32-S3-PCB) | `2c95ed51a500d929fd6d8f519b895f0c4daeb982` | 49 footprints, 559 tracks, 85 vias, 2 zones | Invalid conversion: undefined layers |
| [Adafruit Metro M4 Express](https://github.com/adafruit/Adafruit-Metro-M4-Express-PCB) | `4aafb2ac645c0d93eed7e4fd6cb71c8eef1a3ac6` | 65 footprints, 790 tracks, 156 vias, 6 zones | Invalid conversion: undefined layers |
| [LimeSDR-Mini v1.3](https://github.com/myriadrf/LimeSDR-Mini) | `ade9b0663994bb039c69a5e3c7ce05b7ce727cbe` | 433 footprints, 5,416 tracks, 821 vias, 76 zones, 8 copper layers | Native read-back and DRC baseline completed; semantics unvalidated |

Both Eagle imports report **zero warnings and errors**, yet emit footprint
objects on `UNDEFINED` layers. Native statistics and DRC cannot reload those
boards (exit 3). In Feather's source, example undefined arcs match the four
0.4826 mm wide arcs on Eagle layer 41 (`tRestrict`, copper keepout).
Automatically dropping those objects or remapping them to documentation
layers would lose routing constraints. The wrapper catches this before
scheduling DRC; both originals are retained as importer regressions.

LimeSDR's [source README](https://github.com/myriadrf/LimeSDR-Mini/blob/ade9b0663994bb039c69a5e3c7ce05b7ce727cbe/README.md)
licenses the hardware CC-BY-3.0. It supplies a dense FPGA/USB3/RF example that
broadens the collection beyond hobby MCU boards. Its first KiCad baseline
has **2,481 reported violations and 23 unconnected items**. Several categories
reach KiCad reporting caps; these are reported counts, not an exhaustive
count of defects. Default KiCad rules and conversion differences can explain
some findings; they have not been adjudicated. Treat it as an import/routing
stress candidate, never a DRC-clean reference design or verified training
label. Revisions 1v0–1v3 belong to one family, not four independent examples.

## Admission and labels

`invalid_conversion`, `import_failed` and `baseline_failed` are useful importer
regressions, not difficult routing benchmarks. A wall timeout only supports a
routing-difficulty label after parsing, geometry and source baseline checks.

`imported_unvalidated` means the native file can be read and a baseline was
recorded. `eligible_as_routing_label` remains false. Promotion requires:

- Retain repository, pinned commit, original path, original SHA-256, exact
  licence/attribution and converter version; retain the original PCB and
  companion project/rule files.
- Compare component reference sets, positions/rotations/sides and pad numbers,
  shapes, drills and layer spans. Compare pad-to-net assignments and net names.
- Compare copper-layer count/stackup, closed outline, holes, cutouts, keepouts,
  copper zones and their nets, tracks, arcs and vias. Resolve importer warnings.
- Reconstruct source-equivalent clearances, net classes, widths, via rules,
  differential-pair constraints and length constraints. Record every unsupported
  rule rather than silently accepting KiCad defaults.
- Inspect source-vs-converted manufacturing plots where available; distinguish
  pre-existing source disconnections from conversion losses. Store the native
  DRC baseline separately from pcb-maker's routed result.

Use provenance families for train/validation/test grouping across formats and
revisions. Keep simplicity and conversion quality as separate axes: small
production boards can still supply useful placement/escape examples even if
pcb-maker routes them quickly.

Converter regression tests (timeouts, XML entities, false-clean import report,
and clean DRC not implying semantic fidelity):

```sh
python3 -m unittest discover -s benchmarks/expansion -p test_convert.py -v
```
