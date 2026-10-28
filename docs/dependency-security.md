# Dependency Security Review

Reviewed on 2026-10-29 for Windows x64. This records the locked dependency
snapshot, its remaining findings, and the checks used after remediation.
It is not a claim that the application has no undiscovered vulnerabilities.

## Remediation

| Component | Before | After |
|---|---|---|
| rustls | 0.23.43 | 0.23.46 |
| rustls-webpki | 0.103.14 | 0.103.16 |
| webpki-roots | 1.0.9 | 1.0.11 |
| tauri-plugin-updater | 2.10.1 | 2.14.1 |
| Tauri | 2.11.5 | 2.12.5 |
| zip | 4.6.1 | 9.1.3 |
| Tokio | 1.52.3 | 1.54.1 |
| Vite | 7.3.6 | 7.3.7 |
| Vitest | 3.2.7 | 4.1.12 |
| devalue | 5.8.1 | 5.9.4 |
| nanoid | 3.3.15 | 3.3.20 |
| postcss | 8.5.16 | 8.5.32 |
| source-map-js | 1.2.1 | 1.2.2 |
| undici | 7.28.0 | 7.30.1 |

Vitest 4 removes Tinypool from the test dependency graph. A listener-disposal
test now gives its mock an explicit callable type for Vitest 4's mock types.
The frontend manifest validator and its fixture require Vite 7.3.7. The
production frontend was rebuilt using Node 24.15.0 and npm 11.12.1; its bundled
JavaScript, CSS and image bytes are unchanged.

The updater upgrade brings the compatible Tauri 2.12 dependency family and
ZIP 9 without overriding an incompatible ZIP version. The existing application
update code, settings, HTTPS requirement and signature configuration contract
are unchanged. The updated Tauri dependency graph no longer contains the five
unmaintained `unic-*` crates from the previous Windows build.

Optional lockfile entries were also refreshed: quinn-proto 0.11.19,
webpki-root-certs 1.0.11, and chacha20 0.10.2. This removes known-version and
yanked findings even for currently disabled optional paths. HTTP/3 was not
enabled by this change.

## Audit Results

| Check | Before | After |
|---|---|---|
| npm affected package entries, all dependencies | 12 | 3 low |
| npm critical / high / moderate entries | 2 / 6 / 1 | 0 / 0 / 0 |
| npm affected entries with `--omit=dev` | 1 | 0 |
| Cargo.lock vulnerability advisories | 15 | 0 |
| Cargo.lock unsound / yanked findings | 2 / 1 | 1 / 0 |
| Findings in enabled Windows normal/build dependencies | see previous audit | no RustSec findings |

npm entries include propagated parent-package findings and are not a count of
independent vulnerabilities. cargo-audit 0.22.2 returned exit code 0 for the
full lockfile. Its database was RustSec commit
`40f5be6fb12ed93fc591bf4ab23b7d793b3f2239`, updated on 2026-10-28.
The complete npm audit still returns exit code 1 because the low finding below
is retained. No audit exclusion or blanket allowlist was added.

## Remaining Findings

### deepmerge, development tooling

[GHSA-gcqx-w48m-w2h4](https://github.com/advisories/GHSA-gcqx-w48m-w2h4)
affects deepmerge 4.3.1. No patched release was available at this review.
The three npm low entries are deepmerge itself and the propagated
`@sveltejs/vite-plugin-svelte` / inspector entries.

deepmerge is used by the Svelte development plugin for configuration merging.
The application does not call it directly, and final Rollup module maps confirm
that it is absent from the committed production chunks. Use trusted local
configuration and plugins. Revisit the upstream fix before the next dependency
refresh or binary release; if untrusted configuration becomes an input, this
assessment must be revisited. This is an open upstream finding, not a fixed
vulnerability or a zero-warning audit.

### Other-target Rust dependencies

The full Cargo.lock still reports maintenance notices for `proc-macro-error`,
`zbus_names`, `zvariant` and `zvariant_derive`, plus
[RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html) in
`glib`. None is in the enabled Windows x64 normal/build dependency tree.
Do not apply the Windows assessment to a future Linux or other-target release.

## Verification And Release Boundary

- Complete frontend tests and TypeScript checking pass.
- Production frontend build and frontend manifest tests pass.
- Focused updater/runtime tests and the complete serial Cargo regression pass.
- Final bundle contributor maps and output hashes were rechecked.
- All 181 npm packages and 316 enabled Windows Rust dependencies have declared
  licenses, recorded in the refreshed [inventory](../third-party/dependency-inventory.md).

This change does not build or publish an installer, enable the update service,
or exercise a real signed installation. The authenticated Favorites acceptance,
corresponding-source and full native binary-notice release gates remain open.

## Recheck

Use Node 24.15.0, npm 11.12.1 and the project's supported Rust toolchain.

```powershell
npm audit --json --registry=https://registry.npmjs.org
npm audit --omit=dev --json --registry=https://registry.npmjs.org
cargo audit --file src-tauri/Cargo.lock --json
cargo tree --manifest-path src-tauri/Cargo.toml --locked --target x86_64-pc-windows-msvc --edges normal,build
npm test
npm run typecheck
npm run app:build
cargo test --manifest-path src-tauri/Cargo.toml --locked -- --test-threads=1
```

Primary references: [Rustls handshake advisory](https://rustsec.org/advisories/RUSTSEC-2026-0285.html),
[certificate distrust advisory](https://rustsec.org/advisories/RUSTSEC-2026-0382.html),
[ZIP symlink advisory](https://rustsec.org/advisories/RUSTSEC-2026-0368.html),
[Vitest 4 migration](https://v4.vitest.dev/guide/migration), and
[Tauri updater documentation](https://v2.tauri.app/plugin/updater/).
