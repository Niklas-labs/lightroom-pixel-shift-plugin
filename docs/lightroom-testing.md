# Installing the CI test build in Lightroom Classic

The CI package contains the Lua plugin and the matching Rust engine. It does not
require installing Rust or choosing an engine executable. This is an experimental
linear-DNG build; Lightroom feature acceptance, bounded-memory processing and
improved subpixel motion correction remain separate checks.

## Download and install

1. Open the repository's **Actions** tab and the latest successful **Package
   Lightroom Plugin** run for `main`.
2. Under **Artifacts**, download the package for your system: macOS universal
   (Intel and Apple Silicon), or Windows x86-64. Download the individual
   `SonyPixelShift-<version>-<platform>.zip` artifact and extract it once.
   `SonyPixelShift.lrplugin` is directly inside, alongside installation notes.
   The package SHA-256 is recorded in the workflow job summary.
3. Move `SonyPixelShift.lrplugin` to a permanent location. Keep its `bin`,
   `manifest.json`, and `licenses` contents together.
4. In Lightroom Classic open **File → Plug-in Manager → Add**, and select that
   `.lrplugin` directory. For an update, fully quit Lightroom, replace the entire
   old `.lrplugin` folder, then reopen Lightroom. In Plug-in Manager verify the
   plugin's installed path; remove/re-add that folder if menu/module registration
   remains stale. Avoid merging individual files from different builds.
5. New macOS CI artifacts require Developer ID signing and Apple notarization;
   older artifacts may be signed-only or development builds.
   INSTALL.txt and the plugin manifest report both states separately. If macOS
   blocks the helper, approve that specific downloaded application through
   **System Settings → Privacy & Security**, then retry the operation. The exact
   prompt depends on your OS security policy.

## macOS execution

Development helpers are ad-hoc signed, not Developer ID signed or notarized.
For signed builds, see [OpenBao setup](openbao-signing.md); test downloads without
removing quarantine first. First try **System Settings → Privacy & Security → Open Anyway** after
macOS blocks that specific helper, then retry in Lightroom.

For a downloaded build you trust, a manual fallback is to remove quarantine from
**only its helper executable**. Replace the path below with your installed plugin
location (the diagnostic log records the helper path):

```sh
xattr -d com.apple.quarantine "/full/path/to/SonyPixelShift.lrplugin/bin/sony-pixelshift"
```

No `sudo` or recursive flag is needed for a plugin you own. Do not remove
quarantine from unrelated downloads or disable Gatekeeper globally. The plugin
never runs this command automatically. An attribute-not-found message means
quarantine was not present on that file; it does not diagnose other launch errors.

A log containing `sony-pixelshift ...` followed by a metadata error means the
helper launched: changing quarantine will not fix that metadata error.

Developer ID signing and notarized distribution are tracked in ROADMAP.md.

## Compare Sony and plugin brightness

For a local comparison of the same capture merged by Imaging Edge and the
plugin, follow [the brightness comparison instructions](brightness-comparison.md).
Export both through Lightroom; the diagnostic reads a bounded sampling grid and
produces a small JSON report and preview without uploading the originals.

## First merge

Start with a complete **four-shot Sony a7CR Pixel Shift** capture. Select the
original ARWs in the Library module, then use **Library → Plug-in Extras → Pixel Shift: more detail (4 or 16 RAWs)**. Confirm the proposed filename. The plugin saves beside the source ARWs
(e.g. `DSC01582-DSC01585-PixelShift.dng`), adding a suffix if that name exists.
Sources must be in one folder. The plugin checks the
bundled engine version, passes original paths to it, and imports the DNG only
after successful completion. Lightroom's top-left activity area shows the job
while it runs: preparing/checking, decoding RAW i of N, aligning/merging, writing
DNG and adding to Lightroom. Streamed jobs also show completed output-region
counts while combining and saving. Decode counts reflect started frames; merge/write
use an activity indicator rather than an invented time percentage. Progress ends
before the completion/error dialog. Cancellation is not offered until the engine
supports safe cooperative cancellation. The completion dialog confirms the catalog lookup
and attempts to select the result. If it is hidden, check library filters and the
current collection; the plugin preserves both. A saved DNG remains available if
catalog import fails.

For sixteen-shot captures, choose **Memory: Disk (less RAM)** if RAM is limited.
Native packed sources use about 1.95 GB at 61 MP in RAM mode, or that amount of
scratch disk in disk mode. One-frame decoder/allocator working memory is additional.
Compare both modes on the same selection; output pixels/crop/color should match.
Record the log performance lines and verify scratch cleanup on completion/failure. The plugin doesn't apply Lightroom edits from the selected sources;
it reconstructs their original sensor data.

The denoise menu operates on ordinary RAW bursts. Initial Sony metadata coverage
and real-camera validation are experimental; a failed capture should produce a
clear error and a diagnostic log rather than silently guessing a frame layout.

## Missing menu commands

Commands are registered under **Library → Plug-in Extras** only. The File menu
can show “none defined”; this is expected. In Plug-in Manager, select the actual
`SonyPixelShift.lrplugin` folder, enable it and reload after replacing a build.
If the Library menu is also empty, check the plugin status/error in Plug-in
Manager; this occurs before launching the Rust helper.

## “Could not load toolkit script”

This occurs in Lua module loading before the helper runs, so quarantine and RAW
settings are unrelated. Verify that you extracted the complete `.lrplugin`, quit
Lightroom fully, replace the whole installed folder and reopen. Check Plug-in
Manager's path to avoid loading another copy. The progress module is now named
`PixelShiftProgress.lua`; it must sit alongside `Workflow.lua` and `Info.lua`.
CI loads the assembled ZIP's menu scripts/dependencies in isolation, with no
source-tree fallback, and constructs progress using plain tables. Actual Lightroom
loader behavior still requires host confirmation.

## Reporting a failure

Keep the engine log path and error shown by the plugin. Report the plugin build
revision, Lightroom and OS versions, camera firmware, shot count, RAW compression
setting, and whether your selected originals are online. Logs can contain local
file paths; inspect them before sharing. A completed DNG remains available to
import manually if catalog import fails.

Synthetic CI fixtures verify packaging, reconstruction and protocol behavior.
They do not establish real-camera image quality or replace running inside
Lightroom Classic. Please keep the source ARWs while testing.

## TIFF appearance and Adobe features

Check camera/lens/focal-length EXIF against the source. Standard EXIF is retained
when present, but Lightroom's RAW-only lens profiles may still be unavailable for
TIFF. Do not choose a different lens profile. Proprietary Sony correction data is
not applied or copied into the cropped TIFF.

Our simple sRGB renderer does not reproduce Adobe's RAW profile/tone curve or
source Develop edits. Compare neutral chart patches with matched exposure and
white balance before attributing a duller appearance to lens correction.

The plugin now writes experimental 32-bit float linear DNG. Test Adaptive Color and Adaptive B&W profiles,
AI Denoise and other Enhance features separately on merged linear DNGs; record
Lightroom version, availability and visual results. DNG alone does not guarantee
eligibility. Adaptive presets and AI masks are distinct features and can already
work on rendered files depending on the operation and Lightroom version.

## DNG host acceptance for this build

Start with four real a7CR Pixel Shift ARWs. The result uses `-PixelShift.dng`
(`-NoiseReduced.dng` for ordinary bursts), saved beside the sources. Confirm:

1. Lightroom imports it and Develop renders it without an unsupported/corrupt-file error.
2. Adaptive Color and Adaptive B&W appear, apply successfully and survive reopening.
3. AI Denoise is available and completes. Record your Lightroom version and GPU;
   Adobe's current documentation lists linear DNG as supported, but our output
   has not yet passed this Lightroom host check. Raw Details is a separate
   operation and does not support linear DNG.
4. White balance, exposure recovery, orientation, crop and colors look correct.
   Compare with a source ARW under matched WB/exposure/profile, then check lens
   identification and correction behavior. Proprietary Sony lens corrections are
   not copied/applied; do not assume a correct lens name proves correct geometry.
5. Repeat with sixteen shots only after the four-shot checks pass, including
   detail and moving subjects. Input frames still require several GB; only output is streamed so far.

The raw image stores camera RGB, without baked WB, gamma, sRGB, tone curve or
source Lightroom edits. Float32 preserves negative black noise and headroom.
Only the small preview is rendered to sRGB; allow Lightroom to generate its
Develop preview before comparing appearances. DNG now uses lossless Deflate strips by default. File size depends on content;
uncompressed equivalents are about 0.73 GB at 61 MP and 2.9 GB at 244 MP. Output
regions are streamed, while packed/disk-backed input storage remains a follow-up. See [DNG validation](dng.md).

For the streamed/lossless build, repeat a four-shot merge and verify the DNG's
Develop rendering, Adaptive profiles and AI Denoise. Numerical tests prove sample
parity, not Lightroom host behavior. Keep the performance log: it now records
stage timings, source/region allocation sizes, output bytes and OS peak memory.
The loader/restart check from the earlier build is still pending; install with
Lightroom fully closed and verify the registered plugin path.


### Optional merge diagnostics (build 11)

1. Leave **Save merge diagnostics** unchecked and merge: only the usual DNG,
   job/result cleanup and persistent log should appear.
2. Check it and merge the same inputs. Confirm **Open diagnostics** reveals the
   overlay in Finder/Explorer, and only the DNG is imported into the catalog.
3. Open `overlay.tif` and `reference-fallback.tif` from the sibling
   `<output>.diagnostics` folder. Red overlay cells indicate at least one fallback
   pixel; white full-resolution mask pixels identify exact fallback locations.
   Check registration with the DNG's visible crop, including portrait captures.
4. Inspect `summary.json`: counts refer to the visible crop, so they can be lower
   than the engine log's full-buffer fallback count. Alignment offsets and the
   configured disagreement threshold are recorded under `context`.
5. Repeat with 16 shots and both RAM/Disk settings. Fallback pixels are still on
   the 2x output grid but contain interpolated first-frame detail. Do not treat
   the mask as ground-truth motion segmentation.
6. Confirm the checkbox is remembered. **Close** dismisses the result without
   opening diagnostics. A Finder/Explorer error must not undo a successful import.

The overlay uses the engine's approximate As Shot preview rendering. Compare
fine detail and brightness in Lightroom Develop, not against the overlay.


### External exFAT volumes (build 12)

Merge a 16-shot capture with diagnostics on the external destination. Confirm the
DNG and diagnostic folder are complete and the DNG is imported. Check the log for
`output publication: ExclusiveCopy` when the volume rejects native no-clobber rename.
The compatibility copy needs additional destination space while saving; see
[publication behavior](streaming.md). A failed merge must not replace an existing
image. Compare a successful result with the same capture saved on APFS: image data
and visible dimensions should match. Real-camera external-drive validation remains
a host test; the CI exFAT fixture test proves filesystem behavior on a disk image.
