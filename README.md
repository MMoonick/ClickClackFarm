# 敲敲牧场 / Click Clack Farm

敲敲牧场是一款常驻电脑桌面的挂机生态游戏：保持屏幕亮起让植物持续生产，日常键盘敲击和鼠标点击会触发动物进食，出售成长后的动物，再扩建自己的牧场。

当前公开版本为可完整试玩的 `v1.0` macOS Demo。

## 下载与运行

- 系统要求：macOS 13 或更高版本
- 处理器：Apple Silicon（M1/M2/M3/M4 及后续机型）
- 下载：[ClickClackFarm-macOS-arm64-v1.0.zip](https://github.com/MMoonick/ClickClackFarm/releases/download/v1.0/ClickClackFarm-macOS-arm64-v1.0.zip)

解压后将“敲敲牧场.app”拖入“应用程序”目录并打开。当前 Demo 未购买 Developer ID 签名和 Apple 公证，因此 macOS 可能阻止首次启动。遇到拦截时：

1. 在 Finder 中按住 Control 点击“敲敲牧场.app”，选择“打开”；
2. 如果仍被阻止，进入“系统设置 → 隐私与安全性”，点击“仍要打开”；
3. 按游戏内弹窗指引开启“输入监控”权限，然后重新打开游戏。

游戏只统计键盘按下与鼠标左、右、中键点击的总次数，不读取或保存按键内容、鼠标位置、点击目标、应用名称或窗口标题。存档仅保存在本机。

## Demo 内容

- 五组植物与动物生产线；
- 亮屏生产、输入触发进食、购买和自定义数量出售；
- 部分出售按平均成长值结算，数量与成长值同步减少；
- 主画面随机游走、进食动画、管理页与公告页；
- 本地自动存档和恢复；
- 主窗口关闭后隐藏到后台，重新打开应用即可恢复。

新存档初始提供 `100` 金币、`1` 株三叶草和 `1` 只小白兔。

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

## 当前限制

- macOS 安装包未签名、未公证，也不包含自动更新；
- 没有云同步、遥测或离线收益；
- 首次使用必须手动开启 macOS“输入监控”权限。

## License

代码采用 MIT License，详见 [LICENSE](./LICENSE)。除另有说明外，游戏美术资源不包含在 MIT 授权范围内。
