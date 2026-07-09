#!/usr/bin/env bash
# Install share templates into $pkgdir/usr/share/tsk with FHS paths baked in.
set -euo pipefail

pkgdir="${1:?pkgdir}"
srcdir="${2:?srcdir}"
share="/usr/share/tsk"
repo_share="${srcdir}/share"

sub() {
  sed "s|@TSK_SHARE@|${share}|g; s|@TSK_CMD@|/usr/bin/tsk|g" "$1"
}

install -d "${pkgdir}${share}/hypr/integrations"
install -d "${pkgdir}${share}/waybar"
install -d "${pkgdir}${share}/lib"

# Hypr templates (recursive)
while IFS= read -r -d '' file; do
  rel="${file#"${repo_share}/hypr/"}"
  dest="${pkgdir}${share}/hypr/${rel}"
  install -d "$(dirname "$dest")"
  sub "$file" >"$dest"
done < <(find "${repo_share}/hypr" -type f -print0)

# Waybar snippets
for file in "${repo_share}/waybar/"*; do
  [[ -f "$file" ]] || continue
  sub "$file" >"${pkgdir}${share}/waybar/$(basename "$file")"
done

install -Dm644 "${srcdir}/docs/packaging.md" \
  "${pkgdir}/usr/share/doc/hypr-taskspace/INTEGRATION.md"

install -Dm644 "${srcdir}/packaging/arch/config.toml.example" \
  "${pkgdir}${share}/config.toml.example"

install -d "${pkgdir}/usr/lib/systemd/user"
sub "${repo_share}/systemd/tskd.service" >"${pkgdir}/usr/lib/systemd/user/tskd.service"
