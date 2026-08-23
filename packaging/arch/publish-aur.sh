#!/usr/bin/env bash
# Prepare (and optionally sync) the AUR package from this in-tree PKGBUILD.
# Does not push unless you pass --push.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: packaging/arch/publish-aur.sh [options]

Updates _aur_sha256 and .SRCINFO in packaging/arch/ from the GitHub tag
v$pkgver. The same PKGBUILD is used for in-tree makepkg and the AUR.

Options:
  --sync DIR   Copy PKGBUILD, .SRCINFO, and the .install file into DIR
               (an aur.archlinux.org clone of hypr-taskspace)
  --push       With --sync, commit and push if that clone has changes
  -h, --help   Show this help

Prerequisites:
  1. [workspace.package] version in Cargo.toml matches pkgver
  2. Tag v$pkgver exists on GitHub (git tag v$pkgver && git push --tags)
  3. curl, sha256sum, makepkg

First AUR repo (one-time):
  git clone ssh://aur@aur.archlinux.org/hypr-taskspace.git
  packaging/arch/publish-aur.sh --sync /path/to/hypr-taskspace --push
EOF
}

arch_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$arch_dir/../.." && pwd)
sync_dir=
do_push=0

while [[ $# -gt 0 ]]; do
  case $1 in
    --sync)
      sync_dir=${2:?--sync requires a directory}
      shift 2
      ;;
    --push)
      do_push=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if (( do_push )) && [[ -z $sync_dir ]]; then
  echo "--push requires --sync DIR" >&2
  exit 2
fi

pkgname=$(sed -n 's/^pkgname=//p' "$arch_dir/PKGBUILD" | head -n1)
pkgver=$(sed -n 's/^pkgver=//p' "$arch_dir/PKGBUILD" | head -n1)
pkgrel=$(sed -n 's/^pkgrel=//p' "$arch_dir/PKGBUILD" | head -n1)
cargo_ver=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -n1)
tag="v$pkgver"
tarball="$pkgname-$pkgver.tar.gz"
tarball_url="https://github.com/jsimonrichard/hypr-taskspace/archive/refs/tags/${tag}.tar.gz"

if [[ -z $pkgname || -z $pkgver || -z $pkgrel || -z $cargo_ver ]]; then
  echo "Could not read pkgname / pkgver / pkgrel / Cargo.toml version" >&2
  exit 1
fi
if [[ $pkgver != "$cargo_ver" ]]; then
  echo "pkgver ($pkgver) != Cargo.toml version ($cargo_ver)" >&2
  exit 1
fi
if [[ ! -f $root/LICENSE ]]; then
  echo "Missing LICENSE at repo root (required for license=('MIT'))" >&2
  exit 1
fi
if [[ ! -f $root/Cargo.lock ]]; then
  echo "Missing Cargo.lock (required for cargo --locked/--frozen)" >&2
  exit 1
fi
if [[ ! -f $arch_dir/$pkgname.install ]]; then
  echo "Missing packaging/arch/$pkgname.install" >&2
  exit 1
fi
if ! command -v makepkg >/dev/null; then
  echo "makepkg not found" >&2
  exit 1
fi

echo "Downloading $tarball_url ..."
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
if ! curl -fsSL "$tarball_url" -o "$tmp"; then
  echo "Tag $tag is not on GitHub yet. Create and push it:" >&2
  echo "  git tag $tag && git push origin $tag" >&2
  exit 1
fi

hash=$(sha256sum "$tmp" | awk '{print $1}')
if [[ ${#hash} -ne 64 ]]; then
  echo "Failed to hash tarball" >&2
  exit 1
fi

echo "Setting _aur_sha256=$hash"
tmp_pkg=$(mktemp)
trap 'rm -f "$tmp" "$tmp_pkg"' EXIT
sed "s/^_aur_sha256=.*/_aur_sha256='$hash'/" "$arch_dir/PKGBUILD" >"$tmp_pkg"
cat -- "$tmp_pkg" >"$arch_dir/PKGBUILD"

cp -f "$tmp" "$arch_dir/$tarball"

echo "Writing .SRCINFO ..."
(
  cd "$arch_dir"
  export TSK_AUR_PKGBUILD=1
  makepkg --printsrcinfo >.SRCINFO
)

if ! grep -q "source = .*v${pkgver}\\.tar\\.gz" "$arch_dir/.SRCINFO"; then
  echo ".SRCINFO is missing the GitHub tarball source" >&2
  exit 1
fi
if ! grep -q "$hash" "$arch_dir/.SRCINFO"; then
  echo ".SRCINFO is missing the tarball sha256" >&2
  exit 1
fi

echo "Prepared $arch_dir/PKGBUILD and $arch_dir/.SRCINFO for $pkgname $pkgver-$pkgrel"

if [[ -z $sync_dir ]]; then
  echo "Next: packaging/arch/publish-aur.sh --sync /path/to/aur-$pkgname [--push]"
  exit 0
fi

mkdir -p "$sync_dir"
if [[ ! -d $sync_dir/.git ]]; then
  echo "AUR clone has no .git: $sync_dir" >&2
  echo "  git clone ssh://aur@aur.archlinux.org/$pkgname.git $sync_dir" >&2
  exit 1
fi

install -m644 "$arch_dir/PKGBUILD" "$sync_dir/PKGBUILD"
install -m644 "$arch_dir/.SRCINFO" "$sync_dir/.SRCINFO"
install -m644 "$arch_dir/$pkgname.install" "$sync_dir/$pkgname.install"
if [[ ! -f $sync_dir/.gitignore ]]; then
  printf '%s\n' 'src/' 'pkg/' 'target/' '*.pkg.tar.*' '*.tar.gz' >"$sync_dir/.gitignore"
fi

if (( !do_push )); then
  echo "Copied AUR files into $sync_dir (no commit). Review, then --push."
  exit 0
fi

(
  cd "$sync_dir"
  git add PKGBUILD .SRCINFO "$pkgname.install" .gitignore
  if git diff --cached --quiet; then
    echo "AUR clone already up to date"
    exit 0
  fi
  git commit -m "$pkgname $pkgver-$pkgrel"
  git push
  echo "Pushed to AUR"
)
