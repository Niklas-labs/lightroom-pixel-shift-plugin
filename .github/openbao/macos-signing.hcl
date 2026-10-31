# KV v2: read one value. No list, metadata, writes, or access to other projects.
path "pixelshift-ci/data/macos-signing" {
  capabilities = ["read"]
}
# The runner discards its token immediately after fetching the credentials.
path "auth/token/revoke-self" {
  capabilities = ["update"]
}
