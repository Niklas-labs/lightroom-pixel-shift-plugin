# CI plugin packages

The **Package Lightroom Plugin** workflow runs for pushes to `main` and supports
manual `workflow_dispatch`. It produces two Actions artifacts:

- `SonyPixelShift-<version>-macos-universal.zip`: a ZIP containing a universal Apple Silicon +
  Intel Rust executable and the Lua plugin.
- `SonyPixelShift-<version>-windows-x86_64.zip`: a ZIP containing the Windows x86-64 executable
  with a statically linked C runtime and the Lua plugin.

Each Actions artifact is the distribution ZIP itself. Download the individual
ZIP artifact and extract it once: `SonyPixelShift.lrplugin`, `INSTALL.txt` and
`TESTING.md` are at its root, with no wrapper directory or nested ZIP.
The SHA-256 checksum appears in the packaging job summary; local assembly also
writes a `.zip.sha256` sidecar. Add the resulting `SonyPixelShift.lrplugin`
directory to Lightroom's Plug-in Manager. The engine is discovered relative to
that directory, so there is no separate executable-selection step.

The packages contain a protocol/version manifest, exact source revision, engine
checksum, project license, dependency license inventory and available upstream
license/notice files. They require a native Rust metadata backend; the assembler
refuses an engine reporting an external ExifTool dependency. No Python, Perl,
ExifTool, Rust toolchain or developer PATH is required at runtime.

macOS binaries are assembled with `lipo`. By default they are ad-hoc signed
**development** builds. The optional Developer ID/notarization path retrieves
Apple credentials from OpenBao using GitHub OIDC, not GitHub secrets. See
[OpenBao signing setup](openbao-signing.md) for exact policy, role, credential
import and the `MACOS_SIGNING_ENABLED=true` repository variable. Once enabled,
signing or Apple acceptance failures stop publication; there is no ad-hoc
fallback. The manifest identifies signing/notarization status.

Windows development binaries remain unsigned. For development macOS artifacts,
see [macOS execution](lightroom-testing.md#macos-execution) for explicit per-helper
quarantine instructions. Signed/notarized helper ZIPs need online ticket lookup;
stapled offline containers and real browser-download acceptance remain follow-ups.
The plugin never changes quarantine automatically.

CI uploads with `actions/upload-artifact` v7 and `archive: false`, preserving
our ZIP bytes and their executable permission attributes. Uploading the unpacked
plugin would lose the helper's executable bit. Intel validation downloads with
`actions/download-artifact` v8 and `skip-decompress: true`, then tests the same ZIP
users download. Action revisions are pinned.

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

The current package uses experimental lossless linear DNG with streamed output
regions and packed u16 RAM/disk source stores. The merge dialog offers a memory
choice; scratch disk mode avoids retaining all decoded frames.
TIFF remains available through CLI jobs. Linux package gates also compare DNG
samples/calibration/preview in independent tifffile and LibRaw readers for all
three merge modes. Lightroom host acceptance, compression, tiled processing and
further motion correction remain separate milestones.
