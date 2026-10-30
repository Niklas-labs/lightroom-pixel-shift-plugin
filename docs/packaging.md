# CI plugin packages

The **Package Lightroom Plugin** workflow runs for pushes to `main` and supports
manual `workflow_dispatch`. It produces two Actions artifacts:

- `SonyPixelShift-macos-universal`: a ZIP containing a universal Apple Silicon +
  Intel Rust executable and the Lua plugin.
- `SonyPixelShift-windows-x86_64`: a ZIP containing the Windows x86-64 executable
  with a statically linked C runtime and the Lua plugin.

Each Actions artifact contains the distribution ZIP and a SHA-256 checksum.
GitHub wraps those files in its own download ZIP; extract that wrapper and then
extract the distribution ZIP. Add the resulting `SonyPixelShift.lrplugin`
directory to Lightroom's Plug-in Manager. The engine is discovered relative to
that directory, so there is no separate executable-selection step.

The packages contain a protocol/version manifest, exact source revision, engine
checksum, project license, dependency license inventory and available upstream
license/notice files. They require a native Rust metadata backend; the assembler
refuses an engine reporting an external ExifTool dependency. No Python, Perl,
ExifTool, Rust toolchain or developer PATH is required at runtime.

macOS binaries are assembled with `lipo` and ad-hoc signed for local execution.
They are **not publisher-signed or notarized**. Windows development binaries are
also unsigned. OS download security prompts may require allowing the specific
helper before use. Developer ID signing, hardened runtime, secure timestamps and notarization of a
staple-capable distribution container are tracked release work in ROADMAP.md.
See [macOS execution](lightroom-testing.md#macos-execution) for explicit per-helper
quarantine instructions for trusted development builds; the plugin never changes
quarantine automatically.

CI extracts the actual ZIP into a path containing spaces, Unicode and an
apostrophe, parentheses and ampersand. It checks the manifest/checksum, executes
the embedded engine with an empty PATH, verifies info/result files and appended
launcher logs, performs a synthetic sixteen-shot TIFF merge and checks structured
no-clobber failures. Windows smoke invokes cmd.exe with the same quoting convention
as the Lua launcher. Package builds are gated on the Rust workspace tests, including
the native metadata parser tests; the synthetic merge alone does not exercise
metadata extraction from ARWs. The same universal macOS package is executed natively on Apple Silicon
and Intel runners. This verifies the packaged helper, not Lightroom's SDK host or
real-camera capture accuracy. See [Lightroom testing](lightroom-testing.md).

To assemble locally, first build the matching native release engine (and, for
macOS universal, combine and sign both target builds), then run:

```sh
python3 scripts/package-plugin.py \
  --binary target/universal/sony-pixelshift \
  --platform macos-universal \
  --revision FULL_40_CHARACTER_COMMIT_SHA
python3 scripts/test-package.py --archive-dir dist --platform macos-universal
python3 scripts/test-package-unit.py
```

The assembler invokes `cargo metadata --locked` to capture dependency identities
and notices. `--metadata path.json` can supply previously captured metadata.
Packaging runs on the target OS because it executes the built engine's `info`
command. Output goes to `dist/`; the staged `.lrplugin` is under the platform
subdirectory, and distribution ZIP/checksum files are at the top level.

The current package keeps experimental TIFF output and whole-image merging.
DNG, tiled processing and further motion correction are separate milestones.
