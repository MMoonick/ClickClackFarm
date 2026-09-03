# 敲敲牧场 / Click Clack Farm

敲敲牧场一款会随着你使用电脑而慢慢成长的桌面放置游戏
种下植物，它们会在电脑亮屏时进行光合作用，随着时间持续生长
养只动物，你日常的键盘敲击和鼠标点击，会一点点转化成它的进食与成长
把牧场藏进桌面后台，然后继续做自己的事
让每天的敲敲打打，养活属于你的专属牧场！
如果你在游戏中遇到问题或者有其他的建议，请通过邮件联系我：534257290@qq.com，感谢你的反馈！

## 游戏演示视频

https://github.com/user-attachments/assets/9d5b74da-968c-4ca9-904b-1f85a552034f

## 下载与运行

- 系统要求：macOS 13 或更高版本
- 处理器：Apple Silicon（M1/M2/M3/M4 及后续机型）
- 下载：[ClickClackFarm-macOS-arm64-v1.0.zip](https://github.com/MMoonick/ClickClackFarm/releases/download/v1.0/ClickClackFarm-macOS-arm64-v1.0.zip)

解压后将“敲敲牧场.app”拖入“应用程序”目录并打开。当前 Demo 未购买 Developer ID 签名和 Apple 公证，因此 macOS 可能阻止首次启动。遇到拦截时：

1. 在 Finder 中按住 Control 点击“敲敲牧场.app”，选择“打开”；
2. 如果仍被阻止，进入“系统设置 → 隐私与安全性”，点击“仍要打开”；
3. 按游戏内弹窗指引开启“输入监控”权限，然后重新打开游戏。

游戏只统计键盘按下与鼠标点击的次数，不读取或保存按键内容、鼠标位置等隐私内容。存档仅保存在本机。

## 本地开发

需要 Rust 1.98、Node.js 24.19 和 pnpm 11.19。

```sh
cd apps/desktop
pnpm install --frozen-lockfile
pnpm tauri dev
```

基础检查：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd apps/desktop
pnpm check
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
```

构建 macOS 应用：

```sh
cd apps/desktop
pnpm tauri build --bundles app
```

构建结果位于 `apps/desktop/src-tauri/target/release/bundle/macos/敲敲牧场.app`。

## 代码结构

- `crates/domain/`：纯 Rust 游戏规则与状态转换；
- `crates/content-config/`：动植物与经济参数；
- `apps/desktop/`：Tauri 2 + React 桌面应用、macOS 输入监控、本地存档和运行时美术。

## License

代码采用 MIT License，详见 [LICENSE](./LICENSE)。除另有说明外，游戏美术资源不包含在 MIT 授权范围内。
