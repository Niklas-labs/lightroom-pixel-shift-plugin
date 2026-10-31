# macOS signing through OpenBao and GitHub OIDC

CI retrieves Apple signing credentials using a fresh GitHub OIDC JWT, without a
long-lived Bao token or Apple credentials in GitHub secrets. The private key
still temporarily enters the macOS runner and an isolated keychain so Apple's
`codesign` can use it; this is retrieval, not remote/HSM signing.

## Shared authentication and workload authorization

The [Kalix Bao configuration](https://gitlab.com/NiklasRosenstein/config/-/tree/main/app.rosenstein.cluster.kalix/bao)
is authoritative. `auth-github-actions.tf` owns Actions issuer trust at
**auth/token.actions.githubusercontent.com**, leaving `github.com` available for
interactive GitHub authentication.
`workload-lightroom-pixel-shift.tf` owns the **lightroom-pixel-shift-macos-signing**
role and its policy. [MR !340](https://gitlab.com/NiklasRosenstein/config/-/merge_requests/340)
adds them through the normal plan/apply pipeline. This plugin repository must not
provision or reconfigure the shared authentication mount.

The role restricts admission to the Kalix audience, immutable repository and
owner IDs, main branch, push/manual events, packaging workflow and `macos-signing`
GitHub environment. It grants read access to the single personal signing-identity
record in the Terraform policy and self-token revocation. Tokens have no default
policy and expire within five minutes. Those permissions and lifetime belong to
the workload role; the shared backend can serve other independently scoped roles.

GitHub-hosted macOS runners must reach `https://bao.kalix.cluster.rosenstein.app`
over trusted HTTPS. Bao must reach GitHub's OIDC discovery/JWKS endpoints. This
trusts code merged into main; OIDC cannot protect credentials from malicious
changes an authorized maintainer merges into that workflow.

## Record migration required before enabling signing

The client now logs into the shared backend and workload role above, but its
retrieval/schema code still expects the earlier combined
`pixelshift-ci/data/macos-signing` record. Adapt it to the existing personal
signing-identity schema and separately authorize the notarization credentials
before enabling signing. The Terraform role intentionally does not grant the
legacy path or access to other Apple credentials. Preserve the personal record
and its private key; do not broaden the policy or create a duplicate to bypass
this mismatch.

The current `prepare-signing-secret.py` helper also describes the legacy combined
record and must be adapted with the reader. That schema contains
`certificate_p12_base64`, `certificate_password`, `signing_identity_sha1`,
`notary_key_p8`, `notary_key_id`, and `notary_issuer_id`. A certificate-only `.cer`
is insufficient for signing, and the App Store Connect Team API key for
notarization is distinct from the Developer ID certificate/private-key pair.

After migration, create GitHub environment **macos-signing**, restrict it to
**main**, and set the nonsecret repository variable **MACOS_SIGNING_ENABLED=true**.
Only the macOS packaging job gets `id-token: write`. Reviewer requirements are
optional; the Bao role independently checks the trusted ref/workflow/environment.

## Runner lifecycle and verification

The script requests an audience-bound GitHub token, reads the credential value,
and immediately revokes its short-lived Bao token, including on read/validation
errors. Credentials stay under a mode-0700 directory in RUNNER_TEMP, never in
GITHUB_ENV, outputs, Rust caches or distribution archives.

An ephemeral keychain imports the P12 and selects the exact Developer ID
Application certificate fingerprint. The universal helper is signed with hardened
runtime and a secure timestamp, then the keychain is deleted. `notarytool` must
return **Accepted** before packaging. The receipt is bound to the helper SHA-256
and kept outside build caches. Final cleanup runs even after failures; ephemeral
runner disposal remains the boundary for a lost or forcibly terminated runner.

Once enabled, authentication, identity, signing, revocation or Apple acceptance
failures stop publication. There is no ad-hoc fallback. With the variable unset
or explicitly `false`, clearly labelled development packages still build. Other
values are rejected. The first live signed run must verify the Bao role,
certificate trust chain and Apple account permissions; portable tests do not
establish that acceptance.

ZIPs and standalone Mach-O helpers cannot carry stapled tickets. Apple registers
the signed helper for online Gatekeeper lookup. A stapled DMG/PKG remains a
follow-up for offline verification. Test a normal browser download with quarantine
intact on Apple Silicon and Intel. The plugin never removes quarantine itself.
