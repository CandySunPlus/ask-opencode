# 0008 安装分发：curl 脚本 + release 资产

用户经仓库根 `install.sh`（curl|sh 或本地跑）安装：binary 从 GitHub Release 资产下载（`ask-opencode-<os>-<arch>-<version>.tar.gz`，darwin/linux × aarch64/x86_64，macOS x86_64 无 Actions runner 故该平台退回本地构建），插件脚本与 cmd-gen agent 不打包进资产、按同一 tag 从 raw 拉取以保证与 binary 同版本；脚本不改 `.zshrc`，只打印启用提示。下载的资产与同名 `.sha256` 比对后再解压落盘，校验工具按平台选（darwin `shasum -a 256` / linux `sha256sum`），不匹配即失败且不留半装产物；tar.gz 顶层即二进制 `ask-opencode`，安装后统一清理临时文件。

cmd-gen agent 必装：它是 generate 的默认 agent，缺了装完不能用。默认装进 opencode 全局 agents 目录 `~/.config/opencode/agents/`（`--agent-dir <目录>` / `ASK_OPENCODE_AGENT_DIR` 可覆盖），与插件同一 tag 契约，下载失败即整体失败不留半装。

版本来源默认 `releases/latest` API 取最新非 prerelease tag，API 不可用时报错并提示手动传版本（不静默降级）；`-V <版本>` / `ASK_OPENCODE_VERSION` 显式指定版本即跳过 API。资产下载按 HTTP 状态区分失败：404（平台无匹配资产，当前为 macOS x86_64）打印本地 `cargo build --release` 提示，其余归为下载错误。重复安装幂等覆盖；`--uninstall` 删除 binary 与插件目录且不碰网络，目标不存在也成功退出。

插件目录的选择与启用提示遵循同一个判定：`--plugin-dir` 显式覆盖时插件装进该目录；否则按 oh-my-zsh 惯例目录判定——`$ZSH_CUSTOM` 非空即用它，未设时依次回退 `$ZSH/custom`、`$HOME/.oh-my-zsh/custom`（后者目录真实存在才成立），两者皆无则只装二进制。`$ZSH_CUSTOM` 是 zsh 的 shell 变量，curl|sh 子进程拿不到（env 里只有 `$ZSH`，甚至 `$ZSH` 也可能没导出），所以回退项就是 curl|sh 这条主安装路径的落点，兜底到 omz 标准安装目录 `~/.oh-my-zsh` 是最稳的一条信号。启用提示按插件实际落点选——落在 omz 惯例目录才打「plugins 数组加 ask-opencode」，被 `--plugin-dir` 移走或未装插件时打 source 提示（因为 omz 的 plugins 数组只加载惯例目录，移走后数组提示会失效）。插件脚本在二进制落盘前先拉进临时目录，下载失败即整体失败，不留半装状态。
