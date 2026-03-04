# Open MAD

Open MAD 是一个基于 Rust Workspace 的桌面自动化 Agent 项目。  
当前实现以 macOS 为主要目标平台，前端使用 Dioxus Desktop，后端使用 Axum，通过大模型驱动桌面操作。

## 当前架构

工作区包含以下 crate：

- `mad-ui`：桌面 UI（Dioxus），负责聊天交互与流式展示。
- `mad-server`：Agent 编排与 API（Axum），执行动作、调用模型、管理记忆。
- `mad-core`：底层能力（截图、键鼠输入、可访问性扫描、模型客户端）。
- `mad-skills`：桌面动作 schema 与技能抽象。
- `mad-cli`：命令行入口（交互模式 / 服务模式）。
- `xtask`：开发任务封装（运行、检查、打包）。

## 主要能力

- 指令驱动的桌面控制：点击、输入、组合键、等待、最小化等。
- 支持 `inspect` / `screenshot` 动作，用于多步决策。
- 流式执行链路：UI 通过 `/api/chat/stream` 获取增量结果。
- 可选本地记忆模块（需配置本地 embedding 模型路径）。
- 高风险操作确认流程（UI 弹窗确认后执行）。

## 运行环境

- Rust stable（Edition 2024）
- macOS（项目依赖可访问性与 CoreGraphics）
- Dioxus CLI（打包时需要 `dx` 命令）

可选安装：

```bash
cargo install dioxus-cli
```

## 配置方式

应用启动时会加载多处配置，但不是“就近覆盖”，而是“环境变量优先 + dotenv 不覆盖已存在值”：

1. 进程启动前已存在的环境变量（优先级最高）
2. `dotenvy::dotenv()` 找到的 `.env`（通常是当前工作目录或其父目录）
3. `~/.open-mad/.env`（仅填充尚未设置的键）
4. 可执行文件同目录 `.env`（仅填充尚未设置的键）
5. `~/.open-mad/config.toml`（代码里通过 `env.or(config)` 作为兜底）

也就是说：同名键一旦在前面的来源被设置，后面的来源不会覆盖它。

最小可用配置是提供 API Key：

```bash
DEEPSEEK_API_KEY=your_api_key_here
```

也可配置：

- `MAD_BASE_URL` / `MAD_MODEL`
- `MAD_VISION_MODEL`
- `MAD_SOCKET_PATH`（默认 `~/.open-mad/mad.sock`）
- `MAD_PORT`（仅保留在状态中，当前服务走 Unix Socket）
- `MAD_MAX_STEPS`、`MAD_MAX_WAIT_SECONDS`、各类 timeout 参数
- `memory_model`、`memory_model_path`（`config.toml`，用于记忆模块）

参考模板见仓库根目录 `.env.example`。

## 快速开始

推荐直接用 `xtask`：

```bash
cargo x run
```

等价命令：

```bash
cargo run -p xtask -- run
```

说明：

- `mad-ui` 启动时会在内部拉起 `mad-server`。
- UI 使用 Unix Socket 访问后端（默认 `~/.open-mad/mad.sock`）。

## 开发命令

运行桌面 UI：

```bash
cargo x run
```

运行 CLI（交互模式）：

```bash
cargo run -p mad-cli -- --interactive
```

运行 CLI（服务模式）：

```bash
cargo run -p mad-cli
```

代码检查：

```bash
cargo x check
```

自动修复 fmt + clippy：

```bash
cargo x check --fix
```

## 打包发布（macOS）

打包命令：

```bash
cargo x bundle
```

产物路径：

- `crates/mad-ui/dist/MadUi.app`
- `crates/mad-ui/dist/MadUi_0.1.0_aarch64.dmg`

当前图标配置位于 `crates/mad-ui/Dioxus.toml` 的 `[bundle]`：

- `icon = ["assets/icon.icns", "assets/icon.png"]`

其中 `icon.icns` 用于 macOS app/dmg 图标链路，避免安装后应用图标丢失。

## API 概览

后端路由定义于 `mad-server/src/api/mod.rs`：

- `POST /api/chat`
- `POST /api/chat/stream`（SSE）
- `GET /api/screenshot`

默认通过 Unix Socket 对外提供服务，HTTP 路径用于应用内请求。

## License

MIT
