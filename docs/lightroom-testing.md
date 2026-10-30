# Installing the CI test build in Lightroom Classic

The CI package contains the Lua plugin and the matching Rust engine. It does not
require installing Rust or choosing an engine executable. This is an experimental
TIFF-output build; DNG, bounded-memory processing and improved subpixel motion
correction are separate milestones.

## Download and install

1. Open the repository's **Actions** tab and the latest successful **Package
   Lightroom Plugin** run for `main`.
2. Under **Artifacts**, download the package for your system: macOS universal
   (Intel and Apple Silicon), or Windows x86-64. GitHub downloads an artifact ZIP
   containing the distribution ZIP and its checksum. Extract both layers.
3. Move `SonyPixelShift.lrplugin` to a permanent location. Keep its `bin`,
   `manifest.json`, and `licenses` contents together.
4. In Lightroom Classic open **File → Plug-in Manager → Add**, and select that
   `.lrplugin` directory. When replacing a build, close Lightroom first or use
   Plug-in Manager to reload the updated plugin.
5. These CI development binaries are not publisher-signed/notarized. If macOS
   blocks the helper, approve that specific downloaded application through
   **System Settings → Privacy & Security**, then retry the operation. The exact
   prompt depends on your OS security policy.

## macOS execution

The current development helper is ad-hoc signed, not Developer ID signed or
notarized. First try **System Settings → Privacy & Security → Open Anyway** after
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

## First merge

Start with a complete **four-shot Sony a7CR Pixel Shift** capture. Select the
original ARWs in the Library module, then use **Library → Plug-in Extras → Pixel Shift: more detail (4 or 16 RAWs)**. Choose a new TIFF destination. The plugin checks the
bundled engine version, passes original paths to it, and imports the TIFF only
after successful completion.

Sixteen-shot captures are also available, but the current whole-image engine may
use roughly 7–10 GB RAM. Do the first sixteen-shot test on a machine with enough
free memory. The plugin doesn't apply Lightroom edits from the selected sources;
it reconstructs their original sensor data.

The denoise menu operates on ordinary RAW bursts. Initial Sony metadata coverage
and real-camera validation are experimental; a failed capture should produce a
clear error and a diagnostic log rather than silently guessing a frame layout.

## Reporting a failure

Keep the engine log path and error shown by the plugin. Report the plugin build
revision, Lightroom and OS versions, camera firmware, shot count, RAW compression
setting, and whether your selected originals are online. Logs can contain local
file paths; inspect them before sharing. A completed TIFF remains available to
import manually if catalog import fails.

Synthetic CI fixtures verify packaging, reconstruction and protocol behavior.
They do not establish real-camera image quality or replace running inside
Lightroom Classic. Please keep the source ARWs while testing.
