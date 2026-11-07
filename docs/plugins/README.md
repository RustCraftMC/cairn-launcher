# Cairn 脚本插件

状态：插件系统已就绪，当前插件 API 版本 1。完整接口说明见[技术规范](ENGINE_SPEC.md)。

插件用 JavaScript（ES Modules）编写，由 QuickJS-NG 执行。插件可以：

- 在主页提供小组件，并随时更新其中的数据或整个界面；
- 提供独立面板页（由小组件动作或 `cairn.panels.open` 打开，不进 dock），并动态推送视图与数据；
- 注册命令，供管理页和小组件按钮调用；
- 访问网络、读写文件、运行外部程序（需要授权）；
- 在每次启动游戏前修改 JVM 与游戏参数、Java、内存、窗口、服务器和环境变量（需要授权）；
- 读取实例列表、启动游戏、接收游戏启动和退出事件（需要授权）。

## 安装与启用

1. 在“设置 → 插件”中点“添加”，选“从 zip 安装”或“从文件夹安装”，或者直接把插件 zip／文件夹拖进这个页面。也可以手动把文件夹放进数据目录的 `plugins/` 下（“添加”菜单里的“打开插件目录”可以直接打开它）再点“重新读取”。
2. 在“设置 → 插件”中选中插件，勾选启用。插件申请了权限时，会先列出权限并标出高风险项，确认后才会授予并启动。
3. 插件提供的小组件可以在详情页或“小组件”标签页添加到主页，也会出现在主页组件库的“自定义”分类。

新安装的插件默认停用。安装同一插件的新版本即为更新，旧版本会保留一份，可在“版本”标签一键回退；更新后申请了新的权限，会暂停运行并显示“需要重新授权”。插件声明的设置在“设置”标签修改；“卸载”可选择是否同时删除插件数据。

## 开发插件

最快的起点是模板：在“设置 → 插件”中点“添加 → 新建插件”，或在命令行运行

```bash
cairn plugins new my-tools --name "My tools"
```

会在 `plugins/my-tools/` 生成一个能运行的插件：一个小组件、一个命令、一个设置项，以及：

- `cairn.d.ts`：全部 API 的 TypeScript 类型定义；
- `jsconfig.json`：让 VS Code 等编辑器检查 `main.js` 的 API 调用，拼错方法名、传错参数、用了不存在的事件或补丁字段都会标红。

启用插件后，修改运行中脚本插件的文件，启动器会在约 1.5 秒内自动重新加载它，`console.log` 和报错显示在管理页的“日志”标签。启动器更新后可以用 `cairn plugins types plugins/my-tools` 换上新的类型定义。

## 第一个插件

```text
plugins/
  hello-clock/
    plugin.json
    main.js
```

`plugin.json`：

```json
{
  "id": "example.hello-clock",
  "name": "Hello clock",
  "version": "1.0.0",
  "api": 1,
  "main": "main.js",
  "widgets": [
    {
      "id": "clock",
      "name": "Hello clock",
      "icon": "clock",
      "size": [4, 2],
      "view": [
        { "element": "text", "text": "{data.time}", "size": 32, "weight": "bold" },
        { "element": "button", "text": "Reset", "action": { "type": "command", "command": "reset" } }
      ]
    }
  ],
  "commands": [{ "id": "reset", "title": "Reset the counter" }]
}
```

`main.js`：

```javascript
export function activate(cairn) {
  const show = () => cairn.widgets.setData("clock", { time: new Date().toLocaleTimeString() });
  show();
  setInterval(show, 1000);
  cairn.commands.register("reset", () => {
    cairn.storage.set("resets", cairn.storage.get("resets", 0) + 1);
    return "Counter reset"; // 返回的字符串会显示为通知
  });
}
```

小组件的 `view` 与 YAML 小组件格式相同，`{data.key}` 显示插件用 `setData` 发送的值，未收到时显示“–”。完整示例在 [examples](examples)：

| 示例 | 演示内容 | 权限 |
| --- | --- | --- |
| [hello-clock](examples/hello-clock) | 小组件数据、绑定进度条、开关、输入框、定时器、存储、命令 | 无 |
| [server-status](examples/server-status) | 查询 Minecraft 服务器在线人数，地址和刷新间隔是设置项 | `network` |
| [launch-tuner](examples/launch-tuner) | 按设置修改启动参数，记录上一次游戏结果和崩溃原因 | `launch`、`game` |

这些示例由测试实际加载运行，修改后需要保持测试通过。

## 权限

| 权限 | 允许的操作 | 风险 |
| --- | --- | --- |
| `network` | 向任意网站发送 HTTP/HTTPS 请求，下载文件 | 一般 |
| `fs` | 读写当前用户能访问的任意文件 | 高 |
| `process` | 运行外部程序和命令行，用系统默认程序打开文件 | 高 |
| `launch` | 修改启动参数、Java、内存、窗口、服务器和环境变量 | 高 |
| `instances` | 读取实例列表并启动游戏 | 一般 |
| `game` | 接收游戏启动和退出事件 | 低 |

不需要授权的能力：小组件、命令、定时器、插件自己的数据目录（`plugin-data/<id>/`）与键值存储、日志、通知和打开网页链接。插件拿不到账户令牌、完整命令行或启动器内部对象。

## 运行方式与恢复

所有启用的脚本插件运行在同一个独立的宿主进程里，每个插件有自己的线程和 QuickJS 运行时，互不共享内存。插件卡死、内存超限或宿主崩溃都不会拖垮启动器：

- 单次执行超过 5 秒会被中断，调用方收到错误，插件继续服务其他事件；
- 每个插件的 JS 堆上限为 128 MiB，超出后该插件停止，其他插件不受影响；
- 宿主进程意外退出后自动重启，一分钟内超过 3 次则停止重启并提示；
- 启动器退出或被强制结束时，宿主进程随之退出。

没有启用脚本插件时不会创建宿主进程。如果某个插件导致游戏无法启动，可以用 `CairnLauncher.exe --no-plugins` 启动，本次会话不运行任何脚本插件（声明式小组件照常显示），再到管理页停用它。命令行同样支持 `cairn --no-plugins launch <实例>`，`cairn plugins` 列出插件状态。

插件的启动补丁同时作用于桌面版、命令行和 `--dry-run`，多个插件设置同一项（如内存）会被视为冲突并阻止启动，错误中指出插件名称。

## 尚未实现

- 插件包签名：无法确认插件来自谁；
- 在线插件市场与自动更新，以及多于一个版本的历史回退；
- 操作系统级沙箱（AppContainer 等）：权限检查在宿主进程中执行，能防止插件越权调用 API，但不能防御利用 QuickJS 漏洞的恶意代码，只应启用可信来源的插件。

## 代码位置

| 位置 | 内容 |
| --- | --- |
| `crates/cairn-plugin` | 清单、权限与设置，安装／更新／回退／卸载（`package.rs`），通信协议，宿主进程（QuickJS 运行时、SDK、原生操作），监督器 |
| `crates/cairn-core/src/launch/hook.rs` | 启动补丁的上下文、补丁格式、合并与冲突检查 |
| `crates/cairn-bridge/src/system/plugins.rs` | 插件目录与授权状态、宿主生命周期、事件订阅与投递、请求处理、小组件数据、启动补丁收集 |
| `crates/cairn-bridge/src/system/plugin_admin.rs` | 修改设置、安装／更新／回退／卸载、文件变更后自动重新加载 |
| `crates/cairn-app/src/views/extensions.rs` | 管理页：安装、启用确认、权限、设置、命令、日志、小组件、回退与卸载 |
| `crates/cairn-cli/src/plugins.rs` | 命令行的插件列表与启动补丁 |
| `crates/cairn-plugin/sdk/cairn.d.ts`、`src/template.rs` | API 类型定义与插件模板 |
