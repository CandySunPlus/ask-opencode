# 0008 安装分发：curl 脚本 + release 资产

用户经仓库根 `install.sh`（curl|sh 或本地跑）安装：binary 从 GitHub Release 资产下载（`ask-opencode-<os>-<arch>-<version>.tar.gz`，darwin/linux × aarch64/x86_64，macOS x86_64 无 Actions runner 故该平台退回本地构建），插件脚本不打包进资产、按同一 tag 从 raw 拉取以保证与 binary 同版本；脚本不改 `.zshrc`，只打印启用提示。下载的资产与同名 `.sha256` 比对后再解压落盘，校验工具按平台选（darwin `shasum -a 256` / linux `sha256sum`），不匹配即失败且不留半装产物；tar.gz 顶层即二进制 `ask-opencode`，安装后统一清理临时文件。
