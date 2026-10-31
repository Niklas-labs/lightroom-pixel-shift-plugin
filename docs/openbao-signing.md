# macOS signing through OpenBao and GitHub OIDC

The packaging workflow can fetch signing material from
`https://bao.kalix.cluster.rosenstein.app` using a fresh GitHub OIDC JWT. No Apple
credentials or long-lived Bao token are stored in GitHub secrets. OpenBao KV v2
holds the certificate/private-key pair as a password-protected P12, its password,
and the App Store Connect Team API key used for notarization.

The private key is exportable: it temporarily enters the macOS runner and an
isolated keychain so Apple's `codesign` can use it. This is secret retrieval,
not remote/HSM signing. Protect the OpenBao storage, backups and admin access as
signing credentials. GitHub-hosted macOS runners need HTTPS connectivity to this
address and a trusted TLS certificate. OpenBao must also reach GitHub OIDC
discovery/JWKS endpoints. Do not disable certificate verification.

## 1. Configure OpenBao once, from your authenticated admin session

These commands create dedicated project mounts. Inspect `bao auth list` and
`bao secrets list` first; if these names already exist, check their ownership
rather than overwriting another project's configuration. The auth mount must
support the JWT auth method. An already authenticated `bao` CLI is needed; do
not put an admin token into GitHub or this chat.

```sh
export BAO_ADDR=https://bao.kalix.cluster.rosenstein.app
bao auth enable -path=github-pixelshift jwt
bao write auth/github-pixelshift/config \
  oidc_discovery_url=https://token.actions.githubusercontent.com \
  bound_issuer=https://token.actions.githubusercontent.com
bao secrets enable -path=pixelshift-ci -version=2 kv
bao policy write pixelshift-macos-signing .github/openbao/macos-signing.hcl
bao write auth/github-pixelshift/role/macos-signing @.github/openbao/github-role.json
```

The committed role restricts admission to:

- Audience exactly `https://bao.kalix.cluster.rosenstein.app`.
- Repository `Niklas-labs/lightroom-pixel-shift-plugin`, immutable repository ID
  `1438976219`, and owner ID `343997774`.
- Branch `refs/heads/main`, event `push` or `workflow_dispatch`.
- That repository's `.github/workflows/package.yml` on `main`.
- Subject `repo:Niklas-labs/lightroom-pixel-shift-plugin:environment:macos-signing`.

Its token has no default policy and a five-minute maximum lifetime. Policy allows
reading only `pixelshift-ci/data/macos-signing` and revoking itself. The runner
revokes it immediately after fetching the value, including on read/validation
failures. PRs, forks, other branches/workflows and other environments cannot use
this role. This trusts code committed to main; OIDC does not protect credentials
from malicious changes that an authorized maintainer merges into that workflow.

## 2. Store the Apple credentials

Export the desired **Developer ID Application certificate with its private key**
from Keychain Access as a password-protected P12. A certificate-only `.cer` is
insufficient. Identify the exact certificate using its SHA-1 fingerprint because
multiple certificates can share the same display name:

```sh
security find-identity -v -p codesigning
```

Use the 40-character hash corresponding to the certificate/private-key pair you
exported. The CI importer verifies that exact valid Developer ID identity exists
before using it, so similarly named certificates cannot be selected accidentally.

Create a dedicated **Team API key** with Developer access in App Store Connect
(Users and Access → Integrations → App Store Connect API). Download its `.p8`
once and record its key ID and issuer UUID. This key authenticates notarization;
it is separate from the Developer ID signing certificate.

From the repository checkout on your Mac, prepare a private JSON value. The helper
prompts for the P12 password and writes with mode 0600 without printing secrets.
Use an unused temporary filename outside the checkout:

```sh
python3 scripts/prepare-signing-secret.py \
  --p12 /path/to/pixelshift.p12 \
  --notary-key /path/to/AuthKey_XXXXXXXXXX.p8 \
  --key-id XXXXXXXXXX \
  --issuer-id YOUR_ISSUER_UUID \
  --identity-sha1 YOUR_CERTIFICATE_SHA1 \
  --output /tmp/signing-material-pixelshift.json
bao kv put -mount=pixelshift-ci macos-signing @/tmp/signing-material-pixelshift.json
rm /tmp/signing-material-pixelshift.json
```

The six stored fields are `certificate_p12_base64`, `certificate_password`,
`signing_identity_sha1`, `notary_key_p8`, `notary_key_id`, `notary_issuer_id`.
Never print `bao kv get` output in CI, commit the generated JSON, or upload it as
an artifact. Keep source key exports/backups in your normal secure storage.
KV v2 retains versions; manage old versions during credential rotation as well.

## 3. Enable the GitHub environment and signed builds

1. In repository Settings → Environments, create **macos-signing** and restrict its
   deployment branches to **main**. No environment secrets are needed. Reviewer
   requirements are optional if you want manual approval for access to the key.
2. In Settings → Secrets and variables → Actions → **Variables**, create the
   nonsecret repository variable **MACOS_SIGNING_ENABLED** with value **true**.
3. Run **Package Lightroom Plugin** on **main** (or push to main).

Only the macOS packaging job receives `id-token: write`; Rust tests, Windows and
Intel smoke jobs do not. The script requests an audience-bound GitHub token,
logs into the dedicated Bao JWT role, retrieves exactly one KV v2 value and
revokes its token. Credentials remain under a mode-0700 directory in RUNNER_TEMP,
never in GITHUB_ENV, step outputs, Rust caches or distribution archives.

An ephemeral macOS keychain imports the P12 and permits only the required Apple
tools. After the universal helper is signed with hardened runtime and a secure
timestamp, the keychain is deleted. The signed helper is submitted as a temporary
ZIP to `notarytool`; **Accepted** is required before the final plugin is assembled.
The receipt is bound to the helper's SHA-256. Manifest and installation notes
then identify a Developer ID signed/notarized helper. A final `always()` cleanup
runs before smoke tests/upload, including after failures. Ephemeral runners are
still the cleanup boundary for a lost/forcibly terminated runner.

Once enabled, failures in authentication, missing fields, certificate identity,
signing, token revocation or Apple acceptance fail the job; it never falls back
to ad-hoc signing. With the variable unset or explicitly `false`, existing
**development** packages still build and remain visibly non-notarized. Other
values are rejected. Remove the obsolete Apple GitHub secrets if you previously
added them; this workflow does not read them.

## Verification and remaining host acceptance

Portable tests simulate authentication, token revocation, failures, private file
permissions, certificate selection, cleanup, Apple acceptance and receipt/binary
matching. CI exercises the development path without credentials. The first real
signed run remains necessary to validate your live JWT role, certificate trust
chain and Apple account access. Neither the live role nor credential value can
be provisioned from an environment without your OpenBao admin access.

The final ZIP and standalone Mach-O helper cannot carry a stapled ticket.
Notarization registers the signed helper with Apple; Gatekeeper may need internet
for ticket lookup. A stapled DMG/PKG remains a follow-up for offline verification.
The submitted helper's signature and bytes are preserved when assembling the
`.lrplugin`. Test a normal browser download with quarantine intact on Apple
Silicon and Intel; a mocked or unquarantined CI launch does not establish that
Gatekeeper acceptance. The plugin never removes quarantine automatically.
