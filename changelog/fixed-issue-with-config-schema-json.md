---
type: fix
---
Fixed issue with `config-schema.json` where Taplo (TOML parsing library) would
flag any `aliases` defined as a table but without an explicit
`.enabled = true` as "valid under more than one of the schemas". This affected
downstream tools such as editor extensions showing errors in perfectly valid
config files.
