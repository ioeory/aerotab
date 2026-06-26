#!/usr/bin/env bash
# Build AeroTab Arch package (.pkg.tar.zst) inside an Arch container or on Arch host.
# Usage (repo root): ./tools/build-arch-pkg.sh [version] [arch]
#   arch: x86_64 | aarch64 (default: native uname -m)
# CI calls this after patching pkg/arch/PKGBUILD.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

version="${1:-$(jq -r .version src-tauri/tauri.conf.json)}"
arch="${2:-$(uname -m)}"
case "${arch}" in
  x86_64 | aarch64) ;;
  *)
    echo "unsupported arch: ${arch} (expected x86_64 or aarch64)" >&2
    exit 1
    ;;
esac

native="$(uname -m)"
if [[ "${arch}" != "${native}" ]]; then
  echo "refusing to build ${arch} package on ${native} host (native build only)" >&2
  exit 1
fi

pkg_dir="pkg/arch"
tarball="AeroTab-${version}.tar.gz"

tar -czf "${pkg_dir}/${tarball}" \
  --exclude='node_modules' \
  --exclude='.git' \
  --exclude='target' \
  --exclude='src-tauri/target' \
  --exclude="${pkg_dir}/pkg" \
  --exclude="${pkg_dir}/src" \
  --exclude="${pkg_dir}/*.pkg.tar.zst" \
  --exclude="${pkg_dir}/*.tar.gz" \
  --transform "s,^,AeroTab-${version}/," .

sha256=$(sha256sum "${pkg_dir}/${tarball}" | awk '{print $1}')

sed -i "s/^pkgver=.*/pkgver=${version}/" "${pkg_dir}/PKGBUILD"
sed -i "s/^arch=.*/arch=('${arch}')/" "${pkg_dir}/PKGBUILD"
sed -i "s|^source=.*|source=(\"${tarball}\")|" "${pkg_dir}/PKGBUILD"
sed -i "s/^sha256sums=.*/sha256sums=('${sha256}')/" "${pkg_dir}/PKGBUILD"

cd "$pkg_dir"
makepkg -fs --noconfirm --nocheck
ls -la "aerotab-${version}"-*-"${arch}".pkg.tar.zst
