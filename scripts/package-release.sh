#!/bin/sh
#
# 打发布资产（ADR-0008，T1 #38）：把 release 二进制打成命名契约的 tar.gz 并生成同名 .sha256。
# 供 CI release workflow 与 tests/release_package.rs 共用，钉住与 install.sh 共享的资产形态。
#
# 用法: package-release.sh <os> <arch> <version> <二进制> <输出目录>
#   <os>      darwin | linux
#   <arch>    aarch64 | x86_64
#   <version> 版本号（tag 名，如 v0.1.0）
# 产出:
#   <输出目录>/ask-opencode-<os>-<arch>-<version>.tar.gz         顶层即 ask-opencode
#   <输出目录>/ask-opencode-<os>-<arch>-<version>.tar.gz.sha256  单行 "<hash>  <name>"

set -eu

os="$1"
arch="$2"
version="$3"
binary="$4"
out_dir="$5"

case "$os-$arch" in
  darwin-aarch64|linux-aarch64|linux-x86_64) ;;
  *) echo "package-release.sh: 未知平台/架构: $os/$arch (应属 darwin-aarch64 / linux-aarch64 / linux-x86_64)" >&2; exit 1 ;;
esac

name="ask-opencode-$os-$arch-$version"

# 临时目录统一由 trap 兜底清理，打包中断不留残留。
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT

mkdir -p "$out_dir"
cp "$binary" "$staging/ask-opencode"
tar -czf "$out_dir/$name.tar.gz" -C "$staging" ask-opencode

# 校验工具按平台选（ADR-0008）；在输出目录内运行，使 .sha256 里的 <name> 为资产文件名（无路径前缀）。
if [ "$os" = "darwin" ]; then
  (cd "$out_dir" && shasum -a 256 "$name.tar.gz") > "$out_dir/$name.tar.gz.sha256"
else
  (cd "$out_dir" && sha256sum "$name.tar.gz") > "$out_dir/$name.tar.gz.sha256"
fi
