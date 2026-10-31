# macOS signing through OpenBao and GitHub OIDC

The macOS packaging job Developer ID signs its universal helper and requires
Apple notarization acceptance before publishing. It uses
GitHub OIDC to retrieve the existing personal signing identity from Kalix Bao;
there are no Apple credentials or long-lived Bao tokens in GitHub secrets.
The private key temporarily enters an isolated runner keychain for `codesign`.
This is secret retrieval, not remote/HSM signing.

## Authentication and credential records

The [Kalix Bao configuration](https://gitlab.com/NiklasRosenstein/config/-/tree/main/app.rosenstein.cluster.kalix/bao)
is authoritative. `auth-github-actions.tf` owns issuer trust at
`auth/token.actions.githubusercontent.com`. The workload-specific role is
`lightroom-pixel-shift-macos-signing`; it binds this repository's owner/repository
IDs, main branch, packaging workflow, push/manual events and `macos-signing`
GitHub environment. Tokens expire within five minutes and have no default policy.
The policy allows only the two exact records below and self-token revocation.
[MR !343](https://gitlab.com/NiklasRosenstein/config/-/merge_requests/343) adds the
notarization permission and environment-claim binding. Its normal merge/apply
is required before the live workflow can succeed.

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

The separate notarization record is:

```text
personal/apple/notarization/2TF792U5Y6/22XYSDVB68
```

Its secret-data field is **private_key_pem**. The public **issuer_id** is custom
metadata, returned at `data.metadata.custom_metadata.issuer_id` by the same
KV v2 data read. Key ID **22XYSDVB68** comes from the record path. No metadata
endpoint or listing permission is required. CI validates the issuer UUID and
PKCS#8 PEM, then writes only the inputs needed by Apple's `notarytool` to a
separate private file. The P12 record is not modified or duplicated.

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
audience, reads the two authorized records and immediately revokes its Bao
token, including when either read or validation fails. Files stay under a mode-0700 RUNNER_TEMP
directory, outside GITHUB_ENV, step outputs, build caches and distribution ZIPs.

The keychain imports the P12 for Apple signing tools, signs with hardened runtime
and a secure timestamp, verifies both architectures, then deletes the keychain.
Final cleanup runs before smoke tests/upload even after failures. Disposal of the
ephemeral runner remains the boundary for a lost/forcibly terminated runner.
Authentication, certificate selection, revocation or signing failures stop the
job; there is no ad-hoc fallback. No opt-in repository variable is required.
Windows artifacts remain unsigned. Local assembly still supports development
packages and explicitly records their different status.

## Notarization and publication

After signing, CI submits a temporary ZIP containing the signed universal helper
to `notarytool`, with Key ID 22XYSDVB68 and the issuer UUID from Bao metadata.
Apple status **Accepted** is mandatory. An invalid submission, API failure or
timeout stops publication; CI does not fall back to signed-only or ad-hoc output.
The receipt is bound to the helper SHA-256 and kept in the private RUNNER_TEMP
directory, outside restored Rust caches. The final plugin preserves that exact
signed helper, and its manifest/INSTALL.txt report notarization acceptance.

Local assembly can still make explicitly labelled development or signed-only
packages. They cannot claim notarization without an accepted matching receipt.

ZIPs and standalone Mach-O helpers cannot carry stapled tickets. Notarization
registers the helper for online Gatekeeper lookup; a stapled DMG/PKG remains a
follow-up for offline verification. Test normal browser downloads with quarantine
intact on Apple Silicon and Intel. The plugin never removes quarantine itself.
