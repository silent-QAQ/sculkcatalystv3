# 独立资源中心部署

## 当前生产部署

- 公网入口：`https://res.mcmy.love`
- 源站：`47.110.67.183`，由 Cloudflare DNS/代理接入。
- Caddy 负责 HTTPS、只读 API、静态对象与写接口 Bearer 鉴权。
- Rust API 由 `sculk-resource.service` 托管，只监听 `127.0.0.1:8789`。
- 元数据位于 `/opt/sculk-resource/data/resource-center.json`，对象目录位于 `/opt/sculk-resource/objects`。
- `sculk-resource-backup.timer` 每日备份元数据和对象，备份保留 14 天。
- 管理入口：`https://res.mcmy.love/resource-admin`。管理账号和密码凭证仅保存在浏览器当前会话。

生产环境可复用 `deploy/sculk-resource.service`、`deploy/sculk-resource-backup.service`、
`deploy/sculk-resource-backup.timer`、`deploy/backup-resource-center.sh` 和
`deploy/Caddyfile.resources`。令牌必须只保存在权限受限的服务器配置中，不要写入仓库或
systemd 的 Caddy 环境变量；部分 Caddy 发行版会把服务环境记录到 journal。

资源中心现在通过统一 HTTP/JSON 接口提供七类资源：

| 类型 | `kind` | 目录路径 |
| --- | --- | --- |
| 服务端核心 | `core` | `/api/catalog/cores` |
| 插件 | `plugin` | `/api/catalog/plugins` |
| 玩家皮肤 | `skin` | `/api/catalog/skins` |
| Blockbench 模型 | `bbmodel` | `/api/catalog/bbmodels` |
| UI 贴图 | `ui_texture` | `/api/catalog/ui-textures` |
| Agent Skill | `skill` | `/api/catalog/skills` |
| 插件配置 | `plugin_config` | `/api/catalog/plugin-configs` |

每一类都支持项目和版本的查询、创建、修改、删除。统一解析接口为：

```text
GET /api/v1/resolve?kind={kind}&project={slug}&minecraft={version}&channel=stable
GET /api/v1/download/{kind}/{project}/{version}
```

`minecraft` 仅在 `core` 和 `plugin` 中必填。素材、Skill 和配置版本使用 `formats` 描述格式；Skill 与插件配置还可以使用 `content` 保存经过 SHA-256 校验的内联 bundle。项目可使用 `preview_url`、`license` 和 `target_plugin` 提供预览、授权及插件关联。

插件项目使用 `plugin_category` 分为 `mainstream`（主流）、`open_source`（开源）、`standard`（普通）和 `paid`（付费）。AI 应调用 `GET /api/v1/plugins/search`，服务端固定按照主流 → 开源 → 普通 → 付费排序，而不是依赖调用方自行排序。

## 部署拓扑

主站只负责界面，所有资源目录请求会发送到 `VITE_RESOURCE_API_BASE`。资源服务器运行 Rust 后端保存目录元数据，由 Caddy/Nginx 提供 HTTPS、静态大文件和长期缓存：

```text
主站前端 ── HTTPS JSON ──> resources.example.com/api/* ──> Rust :8789
                               │
                               └── /objects/*（大文件静态直出）
```

## 资源服务器

1. 将项目部署到资源服务器并复制 `.env.resource-center.example` 为 `.env`。
2. 修改 `SCULK_ALLOWED_ORIGINS` 为主站的精确来源，例如 `https://panel.example.com`。
3. 运行后端：

   ```powershell
   Set-Location backend
   cargo run --release
   ```

4. 修改并启用 `deploy/Caddyfile.resources.example`。示例把 `/objects/*` 映射到 `D:/sculk-resource-objects`；目录写接口仅接受与总站一致的 Bearer Token。
5. 打开 `https://resources.example.com/resource-admin`，输入管理账号和密码后即可新建项目、从本机上传文件并发布版本。上传接口会自动生成对象 URL、文件大小和 SHA-256。
6. 外部托管资源也可以继续手工填写 `download_url`。下载 API 会统计次数后返回 HTTP 307 到对象文件或外部源站。

`SCULK_STATE_FILE` 保存全部目录元数据，应定期备份其 JSON、`.bak` 和对象目录。生产环境不要直接公开未鉴权的 POST、PUT、PATCH、DELETE；示例 Caddy 配置只允许持有 `SCULK_RESOURCE_API_TOKEN` 的总站写入。

资源服务器至少需要以下变量，其中两个 Token 必须使用相同的高熵值：

```text
SCULK_CATALOG_ADMIN_TOKEN=<开服器总站等自动化客户端使用的 Rust 写接口令牌>
SCULK_RESOURCE_API_TOKEN=<Caddy 与总站同步令牌>
SCULK_CATALOG_ADMIN_USERNAME=<浏览器管理账号>
SCULK_CATALOG_ADMIN_PASSWORD=<浏览器管理密码>
SCULK_CATALOG_ADMIN_BASIC_AUTH=<账号:密码的 Base64 编码，供 Caddy 匹配 Basic 请求头>
SCULK_RESOURCE_OBJECT_DIR=/opt/sculk-resource/objects
SCULK_RESOURCE_OBJECT_ROOT=/opt/sculk-resource/objects
SCULK_RESOURCE_PUBLIC_BASE=https://resources.example.com
SCULK_RESOURCE_UPLOAD_MAX_BYTES=268435456
```

核心目录可通过多镜像自动补全。资源站设置 `SCULK_MSL_CORE_SYNC_ENABLED=true` 后，会在启动时及每 2 小时检查一次 `SCULK_MSL_TARGET_VERSIONS`。同步器先根据上游版本清单创建不可下载的草稿占位，再依次尝试 MSL、FastMirror、Polars；任一镜像成功解析 HTTPS 制品后，占位才转为已发布版本。目录中已经存在的手工版本或完整镜像版本直接复用，不查询也不替换构建。

镜像接口地址可分别通过 `SCULK_MSL_API_BASE`、`SCULK_FASTMIRROR_API_BASE`、`SCULK_POLARS_API_BASE` 配置。同步器通过 `SCULK_MSL_REQUEST_INTERVAL_MS` 控制 MSL 请求速率，并在遇到 429 或暂时性服务错误时继续尝试后续镜像；仍未补齐的占位会留到下一轮。新解析制品会通过 HEAD 获取文件体积，既有自动镜像版本缺少体积时只回填体积，不查询构建、不改变下载地址。首次实际下载还会把最终体积和 SHA-256 回填到目录。镜像来源：[MSL 开服器](https://www.mslmc.cn/docs/msl/msl-mirrors/)、[FastMirror](https://www.fastmirror.net/#/download/)、[Polars](https://mirror.polars.cc/#/minecraft/core)。

浏览器使用 HTTPS 上的 HTTP Basic 账号密码认证，Caddy 精确匹配完整 Basic 凭证，Rust 后端再解码并校验服务端环境中的账号密码。开服器总站等自动化客户端继续使用 `SCULK_RESOURCE_API_TOKEN` / `SCULK_CATALOG_ADMIN_TOKEN` Bearer 令牌，不受管理页登录方式变化影响。即使有人绕过 Caddy 直接访问源站端口，Rust 仍会拒绝无效凭证。

### 发布更新

拥有服务器 SSH 权限后，可从项目根目录执行：

```powershell
.\scripts\deploy-resource-center.ps1 -RemoteHost 47.110.67.183 -RemoteUser root -InstallCaddyConfig
```

脚本会构建前端、上传带时间戳的发布包、在 Linux 服务器编译 Rust 后端、原子切换 `current` 版本、验证 Caddy 配置并重启服务。`config/resource.env`、目录元数据和对象文件都位于发布目录之外，不会被覆盖。

## 主站前端

构建主站前设置资源 API 地址：

```powershell
$env:VITE_RESOURCE_API_BASE='https://resources.example.com'
Set-Location frontend
pnpm run build
```

未设置该变量时会回退到 `VITE_API_BASE`，再回退到同源地址，因此本地单机模式保持兼容。由于 Vite 在构建时注入变量，修改资源域名后需要重新构建前端。

## 总站自动构建插件 Skill

在开服器总站后端配置：

```text
SCULK_RESOURCE_API_BASE=https://resources.example.com
SCULK_RESOURCE_API_TOKEN=<与资源服务器 Caddy 一致的高熵 Token>
SCULK_RESOURCE_SYNC_INTERVAL_SECONDS=300
GITHUB_TOKEN=<可选>
```

总站空闲且资源库已连接时会执行以下流程：

1. 查询 `plugin_category=mainstream` 的主流插件。
2. 用 `target_plugin` 查询 Skill 库中是否已有已发布的专属 Skill。
3. 缺失时创建本地生成任务；自动化队列空闲后读取 GitHub 源码树，优先分析默认配置、插件声明、配置实现和文档。
4. 调用 `config` 场景绑定的 AI 模型，生成 `SKILL.md`、`agents/openai.yaml`、`references/configuration.md` 和配置模板。
5. 校验 Skill 名称、frontmatter、默认提示词和参考内容，再分别上传 Skill 库与插件配置库。

状态和人工触发接口为 `GET /api/resource-sync/status`、`POST /api/resource-sync/scan`、`POST /api/resource-sync/run-next`。没有配置 AI 模型或源码暂时不可读时，任务会保留为等待状态并记录原因，不会生成臆造配置。

## 接口约定

- `GET /api/catalog/{resource}` 支持 `search`、`minecraft` 和 `channel` 查询参数。
- 插件目录额外支持 `plugin_category`；Skill 与配置目录支持 `target_plugin`。
- `GET /api/v1/plugins/search` 是 AI 插件发现入口，固定应用主流、开源、普通、付费优先级。
- `GET /api/catalog/{resource}/{slug}/versions` 返回版本历史。
- `GET /api/openapi.json` 返回完整 OpenAPI 3.1 文档。
- `POST /api/catalog/admin/verify` 验证浏览器管理账号密码或自动化 Bearer 令牌。
- `POST /api/catalog/admin/upload?kind=...&project=...&version=...&filename=...` 接收二进制文件，写入对象目录并返回 URL、大小和 SHA-256。
- 已发布版本必须提供非零 `size` 与 64 位十六进制 `sha256`。
- 核心/插件版本必须提供 `minecraft_versions` 和 `loaders`；素材、Skill 与配置版本必须提供 `formats`。
- `preview_url`、`download_url` 和对象文件建议全部使用资源服务器自己的 HTTPS 域名，减少跨源和上游可用性问题。
