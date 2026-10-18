# Z-Suite 3.7.0.0 – findings (REA + sample files)

## Application
- `Z-SUITE.exe` (520 KB) is only a .NET apphost. All logic is in `Z-SUITE.dll`
  (58 MB, **managed .NET 8**, x64, IL-only, no ReadyToRun, no strong name).
  REA evidence: `ev_ebdbd1550e15285772b4ef71e7a438ebff29c6038a16f27134ede05c8be26270`.
- UI: WPF + CefSharp (embedded Chromium, `Zortrax.ZSuiteWinWpf.*`, JS bridge `JsProxyAsync`/`JsLibrary`).
- ~2405 types, 17254 methods. 53 MB of the DLL is embedded resources (475 manifest resources).
- Part of the string/type names is obfuscated (random names, `EncryptStrings`), so decompiled code will be partly unreadable.
- Libraries: MessagePack, protobuf-net, Newtonsoft.Json, LibTessDotNet, Poly2Tri, SkiaSharp, SignalR, FluentFTP.
- Namespaces of interest: `Zortrax.IO.Implementation` (CT_*/ST_* = 3MF-like schema:
  Object, Mesh, Components, Slices, Material, Color, Texture, ProtectedResources),
  `Zortrax.IO.Abstracts.IZprojFile`, `Zortrax.Printers.*` (M200Plus, M300Plus, Inkspire2, ActiveceraM, CustomResin),
  `Zortrax.Configuration.*`, `Zortrax.Algorithms.*` (AutoArrange, AutoRotate), `Zortrax.Data.Segments.*`.
- Slicer/export types seen in metadata: `ZCodeX`, `ZCodeX2`, `ZCodeX2Metadata`, `ZCodeX2AdditionalMetadata`,
  `ZCodexVersion`, `ZCodex2Strategy`, `ZCodexPlusStrategy`, `BinaryZCodeFile`, `ZProjFile`, `ZprojFileFactory`,
  `SupportLegacyZCodes`, resin: `GenerateResinZCodexImages`, `FillInResinZCodeX2`.
  (Exact full names of ZProjFile/BinaryZCodeFile not yet resolved: not in `Zortrax.IO.Implementation`.)

## `.zprof` (print profiles, `Profiles/*.zprof`)
- `gzip( .NET BinaryFormatter( Dictionary<string, byte[]> ) )`.
- Keys seen: `Defaults`, `ProjectVersion`; values are nested BinaryFormatter blobs
  (e.g. `Zortrax.Printers.JsonSerializationClasses.DefaultProfileToJson`, fields like `<NozzleDiameter>k__BackingField`).
- Rust: needs a small MS-NRBF (BinaryFormatter) reader. Doable (format is public: MS-NRBF).

## `.zcodex2` (sliced print file) – sample files in `Downloads\*.zcodex2`
Layout (little-endian):
1. `u32 json_len` + UTF-8 JSON metadata (`PrintId`, `ZCodexVersion` "2.0.0.0", `SoftwareVersion`,
   `MinFirmwareVersion`, `PrinterModelEnumId`, `PrinterName`, `Materials[]`, temperatures,
   `CommandCount`, `PrintTime`, `NozzleDiameter`, `PrintBoundingBox` (µm), `LayerThickness`,
   `TotalLayersCount`, `Timestamp`, `ResinMetadata`).
2. `u32 png_len` + PNG preview image.
3. A standard ZIP (Deflate) with 4 entries:
   - `AdditionalMetadata` – JSON (per-layer material usage, ...)
   - `ZCodeData` – **binary command stream** (not text G-code; tagged/compact, delta-style values) – format NOT decoded yet
   - `ConfigurationData` – **looks encrypted/high-entropy** (starts `2c 22 00 00 | 01 00 00 40 | ...`) – NOT decoded
   - `UserSettingsData` – JSON (slicer user settings, support angle, gaps, ...)

## Unknown / next steps
- Decode `ZCodeData` opcodes (needs method bodies of the ZCodeX2 writer/reader; REA gives metadata only, no C# decompilation).
- `ConfigurationData` cipher/key (CryptoStream, RSACryptoServiceProvider, `Encrypt/Decrypt` appear in metadata).
- `.zproj` project container (3MF-like?) – no sample file on this machine yet.
- Full inspection of `Z-SUITE.dll` members via REA exceeds its 64 MB budget; must be queried per exact `type_names`.

---
# Update 2 – decompiled with ilspycmd 8.2 (source in %TEMP%\ilspy\src, strings still obfuscated)

## `ZCodeData` (DECODED, verified on 5 sample files, 2.78 M records, 100 % CRC OK)
Stream of back-to-back records, no header, no footer:
```
u8  len        // number of bytes of the record WITHOUT the final CRC  (record size = len + 1)
u8  type       // 1..22, 26..30   (if type > 127 the whole record, CRC included, is XOR 0xFF – "flipped" variant, never seen in samples)
... payload (len - 2 bytes)
u8  crc8       // CRC-8, poly 0xD5, init 0, no reflect, no xorout, over the first `len` bytes
```
- Number of records == `CommandCount` in the JSON header (20890 for Calibration Cube).
- It is a binary G-code. Mapping seen in the G-code converter (`JlyC7XPMb37mWWVZLKV.yPI4Ke9fY9o`):
  G0 → type 27, G1 → type 1 (+ type 2 for F feedrate), G4 → type 12, G28 → 5, G29 → 29/30, G90 → 9, G91 → 10, G92 → 4(?) / 92 → type 4 (J7YaPf…),
  others (types 7, 8, 11, 14, 16, 19-22, 26, 28) not mapped yet.
- Type 1 (move) payload: `u8 mode` (enum = toolpath role: ModelContour=0, SupportInfill=5, Raft=10, Brim=34, Seam=223, Pause=224, JumpPath=251, RetractionBack=253 …),
  `u8 flags` (bit0..7 = X,Y,Z,E,Fan1,Fan2,Z2,Tmp4 present), then one `i32 LE` per set flag in that order. Units: integer (X/Y/Z scaled, bbox in µm in JSON header).
- Type 2: feedrate, `u32 LE`. Type 27 (G0 rapid): same style as type 1 (X/Y/Z).
Example: `0801 ff 04 983a0000 5f` = len 8, type 1, mode 0xFF(Unknown), flags 0x04 (Z only), Z = 0x3a98 = 15000, crc 0x5f.

## `ConfigurationData` (NOT decodable by design)
`u32 total_len | u32 256 | RSA-2048-OAEP(AES key) | u32 256 | RSA-OAEP(IV) | u32 n | Rijndael-CBC(JSON of Zortrax.Configuration.Configuration)`.
Key/IV are random per file and encrypted with a vendor RSA **public** key embedded in the DLL; the private key is not in the application,
so the slicer config inside a .zcodex2 can't be read (nor needed): a Rust slicer should keep its own config. Whether printer firmware requires
a valid `ConfigurationData` to accept a file is UNKNOWN (needs a test on a printer or firmware analysis).

## Other findings
- `AdditionalMetadata` = `ZCodeX2AdditionalMetadata` (Newtonsoft JSON), header JSON = `ZCodeX2Metadata`, `UserSettingsData` = JSON.
- Resin printers (Inkspire 2): a different writer produces a ZIP of per-layer images (PNG or other bitmaps) + config text; `ConfigData` (JSON ints: LayerTime, BottomLayersTime, ... WiperSpeed).
- Strings in the DLL are encrypted by the obfuscator (`p7oYGLcbW1DI5lpOXbA.oT24QCCWPaF(id)`), so literal names (e.g. the zip entry names) were taken from the sample files instead.
- Helper AES (`Rfc2898DeriveBytes`, 2 iterations) exists for string/settings encryption – unrelated to print files.
- `.zproj` project writer/reader not located yet (obfuscated); no sample file available.

## Container detail
The ZIP central-directory/local-header offsets are absolute in the whole file (header + PNG included), so read the ZIP from the start of the file. See `zcodex2/` (Rust crate, tested on the 5 samples).

---
# Update 3 – remaining record types, string table, project/profile container

## Obfuscator string table (decoded)
`p7oYGLcbW1DI5lpOXbA.oT24QCCWPaF(id)` reads UTF-16 strings from an encrypted managed resource
(`bT8dMVXggXiZFg3Mkm.b9awHiMBuf1viDDo7b`). Cipher: 32-bit-word stream cipher (state += keyword; state += f(state); out = state ^ in),
key built at run time by control-flow-flattened code (decompiler artefact: `byte[] x = default(byte[])` is NOT an assignment).
Reproduced in `zcodex2/tools/` (`strdec.py`, `emu.py`, `subst.py`); `tools/strings.txt` lists the 2 412 strings. `subst.py` rewrites the decompiled source with real strings.

## ZCodeData record kinds (all 27 mapped, see `Command` in the crate)
1 G1 move | 2 F feedrate(i32) | 3 G92 zero axes | 4 G92 set position | 5 G28 home | 7 T select tool | 8 M104/M109 (tool u8, temp i32)
9 G90 | 10 G91 (axis flag byte) | 11 progress(u8) | 12 G4 dwell(u32) | 13 (u8,f32) | 14 M190 bed | 15 (i32,i32) | 16 M191 chamber
17 M101 | 18 raw bytes | 19 M2 end | 20 M110 (u32, empty in samples) | 21 M111 | 22 M192 | 26 M70 | 27 G0 | 28 M900 (f32 K, u8) | 29/30 G29.
Kinds present in the 5 samples are verified byte-for-byte; the others come from the decompiled serializers only. Kinds 6, 18, 20, 29 stay raw in the crate.

## Entry names (from the string table)
Provider classes: `ZCodeMetadata`, `Preview.png`, `ZCodeData`, `ConfigurationData`, `UserSettingsData`, `ConfigData` (resin), `ResinMetadata`,
`ResinGCodeData`, `ResinPreviewData`, `ResinSlicesData`, and `ResinSlicesData/<name>` (blob). The `.zcodex2` writer is the class that writes
`u32+ZCodeMetadata`, `u32+Preview.png`, then a ZIP of the other entries (confirms the layout).
File extensions known to the app: `.zprojx` (current project), `.zproj`, `.3mf`, `.zcodex`, `.zcodex2`, `.zprof`.

## Project / profile container (`.zprof`, and the project store behind `.zproj(x)`)
`gzip( .NET BinaryFormatter( Dictionary<string, byte[]> ) )`, implemented in `zcodex2::Store`
(re-encoding `Profiles/default mode.zprof` reproduces Microsoft's BinaryFormatter bytes exactly).
Value encoding by key: `Models` = protobuf-net; `HollowHoles, HeightAreas, PreSlicedSupportStructures, ResinSupportStructures, ResinLayers,
OriginalResinLayers, SupportCrosses, Bridges` = MessagePack; every other key (`ProjectVersion`, `Printer.Model`, `Defaults`, ...) = nested BinaryFormatter.
UNVERIFIED: that `.zprojx` uses exactly this envelope (no sample file on this machine); the project service uses this store class, so it is very likely.
The value schemas (`Model3` for protobuf, the MessagePack layer types, BinaryFormatter classes) are NOT decoded yet.

---
# Update 4 – project keys and value schemas

## Keys written by the project service (all stored in the `Store` dictionary)
Strings (nested BinaryFormatter `string`): `ProjectVersion` (e.g. "2.7.0.0"), `Printer.Model`, `PrinterId`, `PlatformSizeX/Y/Z`,
`ResolutionWidth/Height`, `CustomResinPrinterFileFormat`.
Numbers/bools: `ResinRemovedEmptyLayersHeight` (double), `PrintSettings`, `SupportCompleted` (bool).
Objects (BinaryFormatter): `Defaults` (`DefaultProfileToJson`: NozzleDiameter, LayerThickness, Quality, InfillName/Id/Density, Active),
`Settings` (`PrintSettingsToJson`/legacy `PrintSettings`), `ResinSettings`, `CurrentProjectState`, `ResinPauses` (List<int>), `ReplicateMoveVectors` (Point3[]).
Byte arrays: `ZCode`, `ZCodeOriginal`, `ZCodePreview` (a finished print file and its preview are embedded in the project).
protobuf-net: `Models` = `List<Model3>`; schema documented in `src/models.rs`
(Model3: 1 vertices Point3{1..3 double}, 2 triangles {1..3 int32}, 3 move Point3, 4 name, 5 unrepaired bool, 7 Guid{1,2 fixed64}).
MessagePack (int-keyed arrays, `[Key(n)]`): `HollowHoles`, `HeightAreas`, `PreSlicedSupportStructures`, `ResinSupportStructures`,
`ResinLayers`, `OriginalResinLayers`, `SupportCrosses`, `Bridges` – element classes are obfuscated; not decoded.

## Crate status
`zcodex2`: `Zcodex2` (container), `Record`/`Command` (ZCodeData), `Store` (gzip + NRBF dictionary), `nrbf::parse` (generic BinaryFormatter reader,
verified on the shipped `.zprof`: `Defaults`, `ProjectVersion`), `Model` + `decode_models`/`encode_models` (protobuf; unit-tested only, no real project to check against).

---
# Update 5 – verified on a real project (`sentinel 2.zprojx`, Z-Suite 3.7)
- `.zprojx` uses exactly the `Store` envelope (gzip + BinaryFormatter `Dictionary<string, byte[]>`); re-encoding is **byte-identical**.
- Keys present in this M200 Plus project: `Printer.Model`, `PrinterId`, `CurrentProjectState` ("Model"), `Models`, `Settings`
  (`PrintSettingsToJson`), `ResinSettings`, `ZCode`/`ZCodeOriginal` (empty), `ZCodePreview` (PNG), `ProjectVersion` ("3.7.0.0").
- `Models` protobuf schema confirmed: 1 model "sentinel 2.stl", 750 vertices, 1552 triangles (indices < 750), translation (100, 100, 1.5);
  re-encoding is byte-identical. protobuf-net omits zero-valued fields (zero doubles/ints, false bool, empty strings, all-zero Guid).
- NRBF reader additions needed by real data: boxed primitives (record 8), library records inside graphs (record 12).
- MessagePack keys (`HollowHoles`, `HeightAreas`, supports, resin layers, `Bridges`) are absent from this project, so still undecoded.

---
# Update 6 – ZProj Studio (`zproj-studio/`)
GUI (egui) + lib: STL import (binary/ASCII, welded), 3D preview, scale/rotate/position, export `.zprojx` using a template project for settings.
Placement rule verified: Z-Suite re-centres vertices on load and applies `Move`; stored vertices are already placed (min Z = 0), `Move` = (x, y, height/2).
Loading the sample project's model and rebuilding it reproduces the same geometry (test `template_model_survives_load_and_rebuild`).
NOT verified: that Z-Suite itself opens a generated project (the envelope/values are byte-compatible with Z-Suite output, but no automated check against the app).

---
# Update 7 – slicer (`zslice/`) and .zcodex2 generation
- Calibration from real files: E is in µm of filament (`filament_mm = length * width * thickness / (pi/4 d²)`), feed in mm/min,
  coordinates in µm, retraction 0.8 mm @ 60 mm/s (+0.01 mm on prime), Z-hop 0.4 mm, layer marker = kind 16 (raft layers negative),
  `M111(3)` for the first 3 layers then `M111(0)`, `M70(1)` at start (+`M70(101)` before the first model layer on a raft),
  `G29{a: first layer µm, b: raft top µm}`, progress marks by elapsed time, epilogue: retract, lift, nozzle off, home XY, Progress(100), end.
- Z-Suite `Settings` object (read from a project) lists the parameters it exposes: see `zslice::Settings` for the ones implemented.
- Slicing the sentinel model with matched settings: time 1h36 vs Z-Suite 1h26, filament 10.3 m vs 8.2 m, 24 vs 25 layers, bounding box within ~2 mm.
- Not verified: printer acceptance, Z-Suite opening the generated `.zcodex2`.

---
# Update 8 – tree supports, support options, G-code converter, slicer integrations
- `zslice::support`: classic + tree supports (branch tips on a grid under overhangs, growth per layer, lean ≤ angle, avoidance of the model,
  attraction + merging), expansion, min area, roof/floor, enforcer/blocker boxes. Tree uses less support material than columns (unit-tested).
- `zslice::import`: G-code (Marlin flavour) -> Z-Suite command stream: roles from `;TYPE:` / `; FEATURE:` / `; feature` comments, G0/G1/G2/G3, G10/G11,
  G4, M104/M109/M140/M190, M106/M107, M82/M83/G90/G91/G92/G20/G21, M221; bounds/temperature checks; layer markers on the Z change that leads to a new layer.
- `zslice::gcode_out`: Z-Suite stream -> G-code (round trip preserves filament within 1 %, bounding box within 1 mm).
- `ztool.exe` (to-zcodex2 / to-gcode / info), usable as PrusaSlicer/OrcaSlicer post-processing script (SLIC3R_PP_OUTPUT_NAME); packs in `integrations/`.
- Not verified: behaviour inside the real slicers (profiles/scripts untested), printer acceptance of converted files.

---
# Update 9 - why Z-Suite rejected generated files, and tree support fixes
- Z-Suite's loader reads `ConfigurationData` with a dictionary indexer (no fallback) and only **stores the bytes** (never decrypts them), so any
  valid entry works for opening. We now generate a real one: JSON of `Zortrax.Configuration.Configuration` (321 public fields, defaults + our values)
  encrypted with Zortrax's RSA-2048 public key (OAEP-SHA1) + AES-256-CBC, same layout as Z-Suite. Earlier (wrong) note: it can be generated.
- `UserSettingsData` must contain all 91 fields of a real file: Z-Suite applies them one by one and looks printer, material group/id, layer thickness
  (0.10/0.15/0.20/0.25/0.30 mm), quality (Draft/Normal/High), infill and surface layers up in its option lists. We start from a real file and only
  override fields with valid values. Non-Z-PLA materials keep Z-PLA's identity there (the header still names the real material).
- `zslice::compat::check` / `ztool check` reproduces these requirements (51 problems on the first generated examples, none now; a real Z-Suite file passes).
- Tree supports: tips are now also sampled along the contour of thin curved overhangs (spheres had none), branches lean towards the nearest neighbour at the
  full allowed angle and merge hierarchically (arch: -62 % support material vs columns, T: -50 %), branches slide away from the model instead of dying.
- Still unverified: Z-Suite actually opening a generated file, printer acceptance.

---
# Update 10 - automatic verification (see VERIFICATION.md)
- Cargo workspace at the root: `cargo test --workspace --release` runs everything; `verifier.ps1` / `verifier.bat` adds regenerated-file checks.
- Every `.zcodex2` is verified before it is written (`zslice::verify`: container bytes, Z-Suite format, command stream physics + header consistency); every `.zprojx`
  too (`project::verify_project`). Invalid files are refused with a precise message.
- Golden format snapshot (`zslice/tests/golden`), randomised settings matrix (`tests/matrix.rs`, `ZSLICE_MATRIX=N`), G-code round trip.
- Found and fixed by it: non-consecutive layer markers + wrong thickness after an empty layer; `Progress` marks before `Progress(0)`; the G-code importer did not prime the filament.

---
# Update 11 - plate limits, red overflow, supports in 3D, simple settings
- Cause of "position (-0.6, 166.9) hors du plateau": raft margin / brim / supports were not clipped to the plate while only the model was checked. They are now
  intersected with the plate (0.6 mm inset); a warning says when the raft was reduced. Regression test `raft_brim_and_supports_never_leave_the_plate`; the matrix gained an "edge" shape.
- 3D view: per-pixel red for the part of the model outside the plate, depth-tested overlay lines for supports / raft / brim (supports sampled every ~1/40 layers), model lifted by the raft height.
- UI: tabs, simple mode (sliders and choices) and advanced mode; the plan (supports...) is computed in the background with a version counter so stale results are dropped.
