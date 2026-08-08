#!/bin/sh
#
# ask-opencode 安装脚本（ADR-0008）：从 GitHub Release 下载发布资产、sha256 校验后装入
# binary 目录。curl|sh 与仓库内 ./install.sh 两条路径行为一致：一切信息只来自参数、
# 环境变量与网络，不依赖脚本自身所在路径或仓库文件。
#
# 用法：
#   curl -fsSL https://raw.githubusercontent.com/CandySunPlus/ask-opencode/main/install.sh | sh
#   ./install.sh [-b <目录>]

set -eu

repo="CandySunPlus/ask-opencode"
api_base="https://api.github.com/repos/$repo"
release_base="https://github.com/$repo/releases/download"

usage() {
  cat <<'EOF'
用法: install.sh [-h] [-b <目录>]

把 ask-opencode 的最新发布二进制装进 binary 目录（默认 ~/.local/bin）。

  -h            显示本帮助
  -b <目录>     二进制目录（默认 ~/.local/bin，环境变量 ASK_OPENCODE_BIN_DIR 可覆盖）
EOF
}

# 从 GitHub API 取最新非 prerelease 的 tag；失败时输出空串。
latest_tag() {
  curl -fsSL "$api_base/releases/latest" |
    sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p'
}

# 按 uname 映射平台/架构（macOS arm64 归一为 aarch64，x86_64 保持不变，ADR-0008）。
map_platform() {
  os="$(uname -s)"
  case "$os" in
    Darwin) os="darwin" ;;
    Linux) os="linux" ;;
    *) echo "install.sh: 不支持的平台: $os" >&2; exit 1 ;;
  esac
  arch="$(uname -m)"
  case "$arch" in
    arm64|aarch64) arch="aarch64" ;;
    x86_64) arch="x86_64" ;;
    *) echo "install.sh: 不支持的架构: $arch" >&2; exit 1 ;;
  esac
}

bin_dir="${ASK_OPENCODE_BIN_DIR:-}"
while getopts "hb:" opt; do
  case "$opt" in
    h) usage; exit 0 ;;
    b) bin_dir="$OPTARG" ;;
    *) usage; exit 2 ;;
  esac
done
[ -n "$bin_dir" ] || bin_dir="$HOME/.local/bin"

map_platform

tag="$(latest_tag)"
[ -n "$tag" ] || { echo "install.sh: 无法获取最新版本（GitHub API 不可用）" >&2; exit 1; }

asset="ask-opencode-$os-$arch-$tag.tar.gz"
asset_url="$release_base/$tag/$asset"
sha_url="$asset_url.sha256"

# 临时目录统一由 trap 兜底清理，校验失败或中断都不留残留。
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/ask-opencode-install.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT

curl -fsSL -o "$tmp_dir/$asset" "$asset_url" || { echo "install.sh: 下载失败: $asset_url" >&2; exit 1; }
curl -fsSL -o "$tmp_dir/$asset.sha256" "$sha_url" || { echo "install.sh: 下载校验文件失败: $sha_url" >&2; exit 1; }

# 校验工具按平台选（ADR-0008）；.sha256 取首行首个字段。
if [ "$os" = "darwin" ]; then
  verify() { shasum -a 256 "$1"; }
else
  verify() { sha256sum "$1"; }
fi
expected="$(sed -n '1s/[[:space:]].*$//p' "$tmp_dir/$asset.sha256")"
actual="$(verify "$tmp_dir/$asset" | sed -n '1s/[[:space:]].*$//p')"
[ -n "$expected" ] && [ "$expected" = "$actual" ] || {
  echo "install.sh: sha256 校验失败，下载已被丢弃" >&2
  exit 1
}

tar -xzf "$tmp_dir/$asset" -C "$tmp_dir"
mkdir -p "$bin_dir"
install -m 0755 "$tmp_dir/ask-opencode" "$bin_dir/ask-opencode"

echo "已安装 ask-opencode $tag 到 $bin_dir/ask-opencode"
