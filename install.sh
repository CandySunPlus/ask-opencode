#!/bin/sh
#
# ask-opencode 安装脚本（ADR-0008）：从 GitHub Release 下载发布资产、sha256 校验后装入
# binary 目录；插件脚本按同一 tag 从 raw 拉取装入插件目录（检测到 $ZSH_CUSTOM 时）。
# curl|sh 与仓库内 ./install.sh 两条路径行为一致：一切信息只来自参数、环境变量与网络，
# 不依赖脚本自身所在路径或仓库文件。
#
# 用法：
#   curl -fsSL https://raw.githubusercontent.com/CandySunPlus/ask-opencode/main/install.sh | sh
#   ./install.sh [-b <目录>] [--plugin-dir <目录>]

set -eu

repo="CandySunPlus/ask-opencode"
api_base="https://api.github.com/repos/$repo"
release_base="https://github.com/$repo/releases/download"
raw_base="https://raw.githubusercontent.com/$repo"

usage() {
  cat <<'EOF'
用法: install.sh [-h] [-b <目录>] [--plugin-dir <目录>]

把 ask-opencode 的最新发布二进制装进 binary 目录（默认 ~/.local/bin）；
检测到 oh-my-zsh（$ZSH_CUSTOM 存在）时，把插件脚本按同一 tag 装进插件目录。

  -h                    显示本帮助
  -b <目录>             二进制目录（默认 ~/.local/bin，环境变量 ASK_OPENCODE_BIN_DIR 可覆盖）
  --plugin-dir <目录>   插件目录（默认 $ZSH_CUSTOM/plugins/ask-opencode）
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
plugin_dir=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    -b)
      [ "$#" -ge 2 ] || { usage; exit 2; }
      bin_dir="$2"; shift 2 ;;
    --plugin-dir)
      [ "$#" -ge 2 ] || { usage; exit 2; }
      plugin_dir="$2"; shift 2 ;;
    --plugin-dir=*) plugin_dir="${1#--plugin-dir=}"; shift ;;
    *) usage; exit 2 ;;
  esac
done
[ -n "$bin_dir" ] || bin_dir="$HOME/.local/bin"

# 插件安装目标（ADR-0008，T3 #40）：--plugin-dir 显式覆盖，否则按 $ZSH_CUSTOM 存在与否决定。
zsh_custom="${ZSH_CUSTOM:-}"
omz_plugin_dir=""
[ -n "$zsh_custom" ] && omz_plugin_dir="$zsh_custom/plugins/ask-opencode"
install_plugin=0
if [ -n "$plugin_dir" ]; then
  install_plugin=1
elif [ -n "$omz_plugin_dir" ]; then
  plugin_dir="$omz_plugin_dir"
  install_plugin=1
fi

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

# 插件脚本在二进制落盘前拉进临时目录，失败不留半装（ADR-0008，T3 #40）。
if [ "$install_plugin" = 1 ]; then
  plugin_url="$raw_base/$tag/zsh/ask-opencode.plugin.zsh"
  curl -fsSL -o "$tmp_dir/ask-opencode.plugin.zsh" "$plugin_url" || {
    echo "install.sh: 插件脚本下载失败: $plugin_url" >&2
    exit 1
  }
fi

tar -xzf "$tmp_dir/$asset" -C "$tmp_dir"
mkdir -p "$bin_dir"
install -m 0755 "$tmp_dir/ask-opencode" "$bin_dir/ask-opencode"

echo "已安装 ask-opencode $tag 到 $bin_dir/ask-opencode"

if [ "$install_plugin" = 1 ]; then
  mkdir -p "$plugin_dir"
  cp "$tmp_dir/ask-opencode.plugin.zsh" "$plugin_dir/ask-opencode.plugin.zsh"
  echo "已安装 zsh 插件到 $plugin_dir/ask-opencode.plugin.zsh"
  # 启用提示按插件实际落点选：plugins 数组只加载 omz 惯例目录（ADR-0008，T3 #40）。
  if [ -n "$omz_plugin_dir" ] && [ "$plugin_dir" = "$omz_plugin_dir" ]; then
    echo "启用：在 ~/.zshrc 的 plugins=(...) 数组里加 ask-opencode"
  else
    echo "启用：在 ~/.zshrc 里 source $plugin_dir/ask-opencode.plugin.zsh"
  fi
else
  echo "未检测到 oh-my-zsh（\$ZSH_CUSTOM 未设置），插件未安装。"
  echo "启用：先取插件脚本再在 ~/.zshrc 里 source："
  echo "  curl -fsSL $raw_base/$tag/zsh/ask-opencode.plugin.zsh -o ~/.ask-opencode.plugin.zsh"
  echo "  echo 'source ~/.ask-opencode.plugin.zsh' >> ~/.zshrc"
fi
