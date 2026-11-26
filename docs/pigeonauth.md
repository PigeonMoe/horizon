# PigeonAuth 混合登录

PigeonAuth 在同一个 Horizon 26.3 世界提供正版、离线两个入口。两个 DNS 名称可以指向同一台主机和同一个 Minecraft 端口。配置的离线名称走密码验证，其他名称要求通过 Mojang 会话验证。离线入口请使用独立的 A 或 AAAA 记录，并在客户端中填入该 DNS 名称。直接填写 IP 会走正版入口。

## 启用

1. 用本仓库的 `Horizon` 分支构建服务端，引擎须包含本分支的登录协议改动。标准构建命令为 `cargo build --release -p horizon-server` 和 `cargo build --release -p horizon-game --no-default-features`。`[engine] path` 指向构建出的 `horizon-client` 可执行文件，资源配置见[主机文档](hosting.md)。
2. 从 [PigeonMoe/PigeonAuth 的 Horizon 分支](https://github.com/PigeonMoe/PigeonAuth/tree/Horizon)安装同机的 `pigeon-auth` 服务。由操作系统的服务管理器向两边注入同一个 `HORIZON_PIGEONAUTH_KEY`。可用 `openssl rand -hex 32` 生成，保存在仅对应服务用户有权读取的配置中。
3. 给 `horizon-server` 设置 `HORIZON_PIGEONAUTH_URL=http://127.0.0.1:38721` 和 `HORIZON_PIGEONAUTH_OFFLINE_HOST=offline.example.com`。`server.toml` 中保留 `[server] online_mode = true`。当前版本使用直连，不能和 Velocity 转发或跨服交接同时启用。混合登录开启时，服务端关闭强制安全聊天，因为离线玩家没有 Mojang 签发的聊天密钥。
4. 启动 PigeonAuth，再启动 Horizon。需要在 Panel 上显示插件时，把插件仓库的 `panel-plugins/pigeonauth.toml` 放到 Horizon 服务端目录的 `panel-plugins/` 下。

## 玩家流程

正版玩家通过会话验证后直接进入游戏。还没有离线密码时，服务端会自动打开设置窗口，可按 Esc 跳过。密码表单走单独的对话框消息，输入无需经过聊天或命令。正版账号及离线密码按 Mojang UUID 关联。改名后使用新名称及原离线密码即可登录，服务端曾见过的旧正版名称也会保留保护。

离线玩家通过配置阶段的对话框注册，再重新输入密码完成登录。认证成功前不会加入世界、装载玩家数据、进入玩家列表或触发进服回调，因此也无法移动、挖掘、打开背包或承受伤害。遇到已存在的正版名称，只有该正版 UUID 已设置离线密码时才会显示登录表单。玩家数据保存在该正版 UUID 下。新注册的离线账号使用独立 UUID，既有无验证离线存档需要由管理员确认归属后迁移。

密码表单需要 26.3 客户端的对话框支持。配套的 Horizon 客户端会遮蔽密码，并在离线注册、登录和等待验证时用黑色背景遮蔽菜单。其他兼容客户端可能显示输入内容。Minecraft 离线连接会加密，但握手无法验证服务器身份。玩家应使用本服务器专用密码。PigeonAuth 或 Mojang 名称查询不可用时，离线入口暂停验证。正版入口继续按 Mojang 原有会话验证流程运行。
