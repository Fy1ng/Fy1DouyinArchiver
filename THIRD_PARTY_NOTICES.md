# Third-Party Notices

Fy1DouyinArchiver's original code is licensed under GPL-3.0-only. Third-party material keeps
its original copyright, license, and attribution requirements. The project
license does not replace those terms.

## Scope Of This Snapshot

The [dependency inventory](third-party/dependency-inventory.md) records the
locked npm packages and the enabled normal/build Rust dependency graph for
`x86_64-pc-windows-msvc`, as reviewed on 2026-10-29. All 181 npm entries
and 316 Rust dependencies declare a license.
This is a source-release inventory, not a certification of a future installer.
Refresh it when either lockfile or the build target changes.

## Bundled Frontend Code

The production JavaScript chunks contain code from the following packages.
Their complete notices are retained in
[frontend-notices.txt](third-party/frontend-notices.txt).

| Package | Version | License used for these notices |
|---|---|---|
| `@tauri-apps/api` | 2.11.1 | MIT, from the Apache-2.0 OR MIT choice |
| `@tauri-apps/plugin-dialog` | 2.7.2 | MIT, from the MIT OR Apache-2.0 choice |
| `clsx` | 2.1.1 | MIT |
| `esm-env` | 1.2.2 | MIT |
| `lucide-svelte` | 1.0.0 | ISC and the included Feather MIT notice |
| `svelte` | 5.56.4 | MIT |

This list comes from the final Rollup chunk module maps, not from assuming that
every npm package is shipped. The analyzed JavaScript agrees with the committed
production output apart from a possible final newline. The inventory records
the production chunk hashes.

The npm dialog package includes its SPDX declaration but not the complete MIT
text. Its notice is supplemented with the MIT text shipped in
`tauri-plugin-dialog` 2.7.2, from the same plugins-workspace release. The original
SPDX declaration is retained as well.

When distributing `dist/` separately from this repository, include its frontend
notices rather than relying on this README or on minified bundle comments.

## Customized Installer Template

`src-tauri/nsis/installer-template.nsi` is based on the Tauri template at commit
`8909f221d1515955fc843808032bdc5d62209c96`. Its upstream MIT license is retained
verbatim in [tauri-template-MIT.txt](third-party/tauri-template-MIT.txt).

- Upstream license: <https://github.com/tauri-apps/tauri/blob/8909f221d1515955fc843808032bdc5d62209c96/LICENSE_MIT>
- Copyright: Tauri Apps Contributors, 2017 - Present.
- Fy1DouyinArchiver's changes implement its package identity and managed uninstall rules;
  the existing provenance block identifies the original template.

## Native Dependency Obligations

The reviewed dependency declarations do not expose an obvious blocker to the
project's GPLv3 choice. This does not waive any upstream obligations. In
particular:

- `cssparser` 0.37.0, `cssparser-macros` 0.7.1, `dtoa-short` 0.3.5,
  `option-ext` 0.2.0, and `selectors` 0.38.0 use MPL-2.0. No secondary-license
  opt-out was found in the inspected package sources outside the standard
  license template. Preserve their notices and source availability obligations;
  MPL-2.0 section 3.3 governs combination with GPL-covered code. Do not relabel
  the upstream files as exclusively GPL-3.0-only.
- `ring` 0.17.14 declares Apache-2.0 AND ISC and includes additional notices for
  BoringSSL and copied code. An AND expression requires both sets of terms, not
  a choice between them.
- `unicode-ident` 1.0.24 requires Unicode-3.0 in addition to its MIT or Apache
  license choice. The ICU-related Unicode-3.0 notices must also be preserved.
- `webpki-roots` 1.0.11 distributes certificate data under CDLA-Permissive-2.0.
  Section 2.1 requires the agreement text to accompany shared data.
- `rusqlite` and its Rust bindings use MIT. The bundled SQLite amalgamation has
  its own public-domain dedication; do not treat that as the license of the Rust
  wrappers or enable different SQLite/SQLCipher features without a new review.

The inventory is based on published package metadata and the available package
sources. Dependency source trees and full native-binary notices are not vendored
by this licensing change. Before publishing an executable, collect the notices
for its actual linked dependencies and provide the complete corresponding
source, build scripts, and required notices in a GPL-compliant distribution.

## Artwork, Platform Content, And System Components

- The application mark in `app/src/assets/app-icon.png`, its built copy, and
  `src-tauri/icons/icon.ico` derive from artwork supplied by the project owner.
  They are project assets, separate from the Lucide/Feather icon set. Do not
  infer permission to use unrelated third-party trademarks from this license.
- Douyin account data, video, audio, covers, and other retrieved content are not
  licensed by Fy1DouyinArchiver's GPL declaration. Their respective rights remain with the
  relevant rightsholders.
- System Edge, Chrome, and WebView2 keep their own terms. They are not relicensed
  under GPL. If an installer carries the WebView2 bootstrapper or other external
  components, separately verify their redistribution terms and notices.

## GPL Text Source

The root [LICENSE](LICENSE) is the unmodified GPL version 3 text from the
[SPDX license-list-data copy](https://github.com/spdx/license-list-data/blob/main/text/GPL-3.0-only.txt).
The project's version choice is explicitly `GPL-3.0-only`; the example wording
in the standard license's application guidance does not add an "or later" grant
to this project.
