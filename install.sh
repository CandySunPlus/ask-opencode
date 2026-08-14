#!/bin/sh
#
# ask-opencode 安装脚本（ADR-0008）：从 GitHub Release 下载发布资产、sha256 校验后装入
# binary 目录；插件脚本与 cmd-gen agent 按同一 tag 从 raw 拉取，分别装入插件目录
# （检测到 oh-my-zsh 时）与 opencode 全局 agents 目录。
# curl|sh 与仓库内 ./install.sh 两条路径行为一致：一切信息只来自参数、环境变量与网络，
# 不依赖脚本自身所在路径或仓库文件。
#
# 用法：
#   curl -fsSL https://raw.githubusercontent.com/CandySunPlus/ask-opencode/main/install.sh | sh
#   ./install.sh [-h] [-V <版本>] [-b <目录>] [--plugin-dir <目录>] [--agent-dir <目录>] [--uninstall]

set -eu

repo="CandySunPlus/ask-opencode"
api_base="https://api.github.com/repos/$repo"
web_latest="https://github.com/$repo/releases/latest"
release_base="https://github.com/$repo/releases/download"
raw_base="https://raw.githubusercontent.com/$repo"

usage() {
  cat <<'EOF'
用法: install.sh [-h] [-V <版本>] [-b <目录>] [--plugin-dir <目录>] [--agent-dir <目录>] [--uninstall]

把 ask-opencode 的发布二进制装进 binary 目录（默认 ~/.local/bin）；把 cmd-gen agent 按同一
tag 装进 opencode 全局 agents 目录（默认 ~/.config/opencode/agents）；检测到 oh-my-zsh
（$ZSH_CUSTOM 或其惯例默认 $ZSH/custom）时，把插件脚本按同一 tag 装进插件目录。重复安装幂等覆盖。

  -h                    显示本帮助
  -V <版本>             指定安装版本（默认最新非 prerelease，环境变量 ASK_OPENCODE_VERSION 可覆盖）
  -b <目录>             二进制目录（默认 ~/.local/bin，环境变量 ASK_OPENCODE_BIN_DIR 可覆盖）
  --plugin-dir <目录>   插件目录（默认 $ZSH_CUSTOM/plugins/ask-opencode，$ZSH_CUSTOM 未设时取 $ZSH/custom）
  --agent-dir <目录>    cmd-gen agent 目录（默认 ~/.config/opencode/agents，环境变量 ASK_OPENCODE_AGENT_DIR 可覆盖）
  --uninstall           删除二进制、插件目录与 agent 文件，目标不存在也成功退出
EOF
}

# 取最新非 prerelease 的 tag。优先 GitHub API 的 releases/latest；API 被未认证限流
# 拒掉（HTTP 403）等失败时，回退到 github.com 的 HTML releases/latest 重定向——该端点
# 走页面 CDN、不受 API 配额限制，从 Location 重定向目标里剥出 tag。两路都失败输出空串。
latest_tag() {
  tag="$(curl -fsSL "$api_base/releases/latest" 2>/dev/null |
    sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
  if [ -n "$tag" ]; then
    printf '%s\n' "$tag"
    return 0
  fi
  curl -fsSL -o /dev/null -w '%{url_effective}' "$web_latest" 2>/dev/null |
    sed -n 's#.*/releases/tag/\([^/?]*\).*#\1#p'
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
agent_dir="${ASK_OPENCODE_AGENT_DIR:-}"
version="${ASK_OPENCODE_VERSION:-}"
uninstall=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    -V)
      [ "$#" -ge 2 ] || { usage; exit 2; }
      version="$2"; shift 2 ;;
    -b)
      [ "$#" -ge 2 ] || { usage; exit 2; }
      bin_dir="$2"; shift 2 ;;
    --plugin-dir)
      [ "$#" -ge 2 ] || { usage; exit 2; }
      plugin_dir="$2"; shift 2 ;;
    --plugin-dir=*) plugin_dir="${1#--plugin-dir=}"; shift ;;
    --agent-dir)
      [ "$#" -ge 2 ] || { usage; exit 2; }
      agent_dir="$2"; shift 2 ;;
    --agent-dir=*) agent_dir="${1#--agent-dir=}"; shift ;;
    --uninstall) uninstall=1; shift ;;
    *) usage; exit 2 ;;
  esac
done
[ -n "$bin_dir" ] || bin_dir="$HOME/.local/bin"
# cmd-gen agent 是 generate 的默认 agent，缺了装完不能用，故必装（ADR-0008）。
[ -n "$agent_dir" ] || agent_dir="$HOME/.config/opencode/agents"

# 插件安装目标（ADR-0008，T3 #40）：--plugin-dir 显式覆盖，否则按 oh-my-zsh 存在与否决定。
# $ZSH_CUSTOM 是 zsh 的 shell 变量，curl|sh 子进程拿不到（env 里只有 $ZSH）；
# 未设时回退 $ZSH/custom——omz 的惯例默认值；$ZSH 也未导出时再回退到 omz 标准安装目录
# $HOME/.oh-my-zsh/custom（只要 custom 目录真实存在就认定装了 omz）。
zsh_custom="${ZSH_CUSTOM:-}"
if [ -z "$zsh_custom" ] && [ -n "$ZSH" ] && [ -d "$ZSH/custom" ]; then
  zsh_custom="$ZSH/custom"
fi
if [ -z "$zsh_custom" ] && [ -d "$HOME/.oh-my-zsh/custom" ]; then
  zsh_custom="$HOME/.oh-my-zsh/custom"
fi
omz_plugin_dir=""
[ -n "$zsh_custom" ] && omz_plugin_dir="$zsh_custom/plugins/ask-opencode"
install_plugin=0
if [ -n "$plugin_dir" ]; then
  install_plugin=1
elif [ -n "$omz_plugin_dir" ]; then
  plugin_dir="$omz_plugin_dir"
  install_plugin=1
fi

# --uninstall：不碰网络，删完即退。
if [ "$uninstall" = 1 ]; then
  if [ -e "$bin_dir/ask-opencode" ]; then
    rm -f "$bin_dir/ask-opencode"
    echo "已删除 $bin_dir/ask-opencode"
  fi
  if [ -n "$plugin_dir" ] && [ -d "$plugin_dir" ]; then
    rm -rf "$plugin_dir"
    echo "已删除插件目录 $plugin_dir"
  fi
  if [ -e "$agent_dir/cmd-gen.md" ]; then
    rm -f "$agent_dir/cmd-gen.md"
    echo "已删除 agent $agent_dir/cmd-gen.md"
  fi
  echo "已卸载 ask-opencode"
  exit 0
fi

map_platform

# 版本来源：-V/ASK_OPENCODE_VERSION 显式指定即跳过 releases/latest（ADR-0008）。
# API 与 HTML 重定向两路都拿不到版本时按错误退出并提示手动传版本。
if [ -n "$version" ]; then
  tag="$version"
else
  tag="$(latest_tag)"
  [ -n "$tag" ] || {
    echo "install.sh: 无法获取最新版本（GitHub API 与 releases/latest 重定向均不可用）" >&2
    echo "install.sh: 请用 -V <版本> 或环境变量 ASK_OPENCODE_VERSION 手动指定版本" >&2
    exit 1
  }
fi

asset="ask-opencode-$os-$arch-$tag.tar.gz"
asset_url="$release_base/$tag/$asset"
sha_url="$asset_url.sha256"

# 临时目录统一由 trap 兜底清理，校验失败或中断都不留残留。
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/ask-opencode-install.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT

# 用 -w 拿状态码而非 -f 硬退，才能区分 404 与网络错误：404 按平台无匹配资产处理（ADR-0008）。
if ! http_code="$(curl -sSL -o "$tmp_dir/$asset" -w '%{http_code}' "$asset_url")"; then
  http_code="${http_code:-000}"
fi
if [ "$http_code" != "200" ]; then
  if [ "$http_code" = "404" ]; then
    echo "install.sh: 平台 $os/$arch 暂无发布资产（$asset_url 返回 404）" >&2
    if [ -n "$version" ]; then
      echo "install.sh: 请先确认版本 $tag 正确；macOS x86_64 等无预编译资产的平台请本地构建：cargo build --release，再把 target/release/ask-opencode 放进 PATH" >&2
    else
      echo "install.sh: 请本地构建安装：cargo build --release，再把 target/release/ask-opencode 放进 PATH" >&2
    fi
    exit 1
  fi
  echo "install.sh: 下载失败（HTTP $http_code）: $asset_url" >&2
  exit 1
fi
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

# cmd-gen agent 与插件同一 tag 契约：从 raw 拉取，失败即整体失败，不留半装（ADR-0008）。
agent_url="$raw_base/$tag/.opencode/agents/cmd-gen.md"
curl -fsSL -o "$tmp_dir/cmd-gen.md" "$agent_url" || {
  echo "install.sh: cmd-gen agent 下载失败: $agent_url" >&2
  exit 1
}

tar -xzf "$tmp_dir/$asset" -C "$tmp_dir"
mkdir -p "$bin_dir"
install -m 0755 "$tmp_dir/ask-opencode" "$bin_dir/ask-opencode"
echo "已安装 ask-opencode $tag 到 $bin_dir/ask-opencode"

mkdir -p "$agent_dir"
install -m 0644 "$tmp_dir/cmd-gen.md" "$agent_dir/cmd-gen.md"
echo "已安装 cmd-gen agent 到 $agent_dir/cmd-gen.md"

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
  echo "未检测到 oh-my-zsh（\$ZSH_CUSTOM 与 \$ZSH/custom 都不可用），插件未安装。"
  echo "启用：先取插件脚本再在 ~/.zshrc 里 source："
  echo "  curl -fsSL $raw_base/$tag/zsh/ask-opencode.plugin.zsh -o ~/.ask-opencode.plugin.zsh"
  echo "  echo 'source ~/.ask-opencode.plugin.zsh' >> ~/.zshrc"
fi
