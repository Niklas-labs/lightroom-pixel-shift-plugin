# macOS signing through OpenBao and GitHub OIDC

The macOS packaging job always Developer ID signs its universal helper. It uses
GitHub OIDC to retrieve the existing personal signing identity from Kalix Bao;
there are no Apple credentials or long-lived Bao tokens in GitHub secrets.
The private key temporarily enters an isolated runner keychain for `codesign`.
This is secret retrieval, not remote/HSM signing.

## Authentication and signing record

The [Kalix Bao configuration](https://gitlab.com/NiklasRosenstein/config/-/tree/main/app.rosenstein.cluster.kalix/bao)
is authoritative. `auth-github-actions.tf` owns issuer trust at
`auth/token.actions.githubusercontent.com`. The workload-specific role is
`lightroom-pixel-shift-macos-signing`; it binds this repository's owner/repository
IDs, main branch, packaging workflow, push/manual events and `macos-signing`
GitHub environment. Tokens expire within five minutes and have no default policy.

The client reads this KV v2 record:

```text
personal/apple/signing-identities/2TF792U5Y6/developer-id-application/01162bea66636d78e09580172b23cf922d71fe04e2fc8879b827dc98c6091960
```

Its fields are **p12_base64** and **p12_password**. The HTTP API uses
`personal/data/apple/...`; the UI's `show` segment is not part of the API path.
The P12 must contain the signing certificate and matching private key. CI derives
codesign's SHA-1 identity from the certificate whose DER SHA-256 matches the
record path, then checks that it is a valid Developer ID Application identity
for team `2TF792U5Y6`. Display names cannot select a different certificate.

The existing secret does not need re-uploading or conversion. For a future local
P12 export, `scripts/prepare-signing-secret.py` can generate the two-field JSON
with mode 0600 and an interactive password prompt; it never prints the value.
Terraform owns the policy, not the secret values. The plugin repo does not
provision or reconfigure the shared auth mount.

GitHub-hosted runners must reach Bao over trusted HTTPS, and Bao must reach
GitHub OIDC discovery/JWKS endpoints. The role trusts code merged into main;
OIDC cannot protect credentials from malicious changes an authorized maintainer
merges into that workflow.

## Runner lifecycle

Only the macOS packaging job gets `id-token: write`. It requests the Kalix
audience, reads the single authorized record and immediately revokes its Bao
token, including on validation errors. Files stay under a mode-0700 RUNNER_TEMP
directory, outside GITHUB_ENV, step outputs, build caches and distribution ZIPs.

The keychain imports the P12 for Apple signing tools, signs with hardened runtime
and a secure timestamp, verifies both architectures, then deletes the keychain.
Final cleanup runs before smoke tests/upload even after failures. Disposal of the
ephemeral runner remains the boundary for a lost/forcibly terminated runner.
Authentication, certificate selection, revocation or signing failures stop the
job; there is no ad-hoc fallback. No opt-in repository variable is required.
Windows artifacts remain unsigned. Local assembly still supports development
packages and explicitly records their different status.

## Notarization remains separate

The signing record contains no App Store Connect API credentials. CI therefore
publishes a **Developer ID signed but not notarized** helper, reflected in
`manifest.json`, INSTALL.txt and the job summary. Signing alone does not remove
normal Gatekeeper rejection after a browser download.

Apple notarization requires a separately authorized Team API key (`.p8`), key ID
and issuer ID. Do not add unrelated credentials to the existing signing record
or broaden its policy. The `notarize` CLI helper accepts a separate private
`notary-credentials.json` with `notary_key_p8`, `notary_key_id` and
`notary_issuer_id`; it is not called by current CI. A future Bao policy/fetch path
must supply that record before enabling automatic notarization. It requires
Apple status **Accepted** and binds the receipt to the helper SHA-256. The package
assembler marks notarization true only for an accepted matching receipt.

ZIPs and standalone Mach-O helpers cannot carry stapled tickets. A stapled DMG/PKG
remains a follow-up for offline verification. After notarization is configured,
test normal browser downloads with quarantine intact on Apple Silicon and Intel.
The plugin never removes quarantine itself.
