# TUI 应用开发深度指南：从 zellij-workbench 到 lazygit

本文以本仓库（`zellij-workbench`，Rust + [ratatui](https://ratatui.rs)）和
[lazygit](https://github.com/jesseduffield/lazygit)（Go，自己 vendor 了一份
[gocui](https://github.com/jesseduffield/lazygit/tree/master/pkg/gocui) 并基于
[tcell](https://github.com/gdamore/tcell) 实现）为两个真实案例，拆解终端交互应用
（TUI）的通用开发套路：事件循环怎么设计、渲染怎么做到高性能、布局怎么组织、以及一个
"好用"的 TUI 到底需要哪些必要功能。

文中所有代码引用都能在对应仓库里找到出处：`zellij-workbench` 的引用直接对应
`src/tui.rs`；lazygit 的引用标注了具体文件路径和大致行号（基于
[jesseduffield/lazygit](https://github.com/jesseduffield/lazygit) 某次 clone 时的
版本，行号可能随上游变化略有偏移，但结构性结论是稳定的）。

## 目录

1. [两大架构范式：即时模式 vs Elm 架构](#1-两大架构范式即时模式-vs-elm-架构)
2. [事件循环：TUI 的心脏](#2-事件循环tui-的心脏)
3. [渲染性能：即时模式不等于暴力全屏重画](#3-渲染性能即时模式不等于暴力全屏重画)
4. [布局系统：约束驱动，而不是像素计算](#4-布局系统约束驱动而不是像素计算)
5. [状态与导航：从一个枚举到一个上下文栈](#5-状态与导航从一个枚举到一个上下文栈)
6. [异步与响应性：永远不要阻塞事件循环](#6-异步与响应性永远不要阻塞事件循环)
7. [必要功能清单：一个"好用"的 TUI 需要什么](#7-必要功能清单一个好用的-tui-需要什么)
8. [案例对照表](#8-案例对照表)
9. [延伸阅读](#9-延伸阅读)

---

## 1. 两大架构范式：即时模式 vs Elm 架构

做 TUI 之前先要想清楚一件事：你的 UI 状态和渲染逻辑之间是什么关系。业界基本收敛到
两条路：

**即时模式（Immediate Mode）**——ratatui 官方文档说得很直接：「UI 在每一帧都被
重新创建，你根据当前状态从头"画"一遍，没有常驻的 widget 对象」。典型形态：

```rust
loop {
    terminal.draw(|f| {
        if state.condition {
            f.render_widget(SomeWidget::new(), layout);
        } else {
            f.render_widget(AnotherWidget::new(), layout);
        }
    })?;
}
```

`zellij-workbench` 的 `draw_tui`（`src/tui.rs`）就是这个模式：`terminal.draw(|frame| {...})`
里每次都重新构建 `List`、`Paragraph`、`Line` 等 widget，不持有上一帧的 widget 引用。
好处是简单直接——UI 逻辑就是状态的直接投影，不用操心"widget 内部状态和 App 状态没同步"
这种 bug；代价是渲染循环和事件循环都得自己管，库不会替你决定"什么时候该重画"。

**Elm 架构（Model-Update-View，MVU）**——Go 生态的
[bubbletea](https://github.com/charmbracelet/bubbletea)（4.3 万+ star）是这个流派
的代表。核心是三个函数：

- `Init() Cmd` — 返回启动时要跑的初始命令
- `Update(Msg) (Model, Cmd)` — 收到消息后返回新状态 + 可选的副作用命令
- `View() string` — 纯函数，只读 Model，渲染出字符串

数据单向流动：`事件 → Msg → Update → 新 Model → View → 终端`，副作用（HTTP、定时器、
子进程）被隔离进 `Cmd`，执行完再变成一条 `Msg` 送回 `Update`。这套东西本质上是把
"如何安全地把异步结果并入状态"这件事，用类型系统固化成了框架契约。

**gocui/lazygit 是第三种、更"实用主义"的形态**：`View`（`pkg/gocui/view.go`）是一个
**持久对象**，持有自己的行缓冲区（`lines []lineType`）、光标、滚动位置——这更接近
传统 retained-mode GUI 的"控件"概念。但它不是等事件触发再局部更新属性，而是通过一个
`tainted` 脏标记 + 显式的 `SetContent`/`Write` 调用来驱动局部重建，本质上是"有状态的
即时模式"：视图对象常驻，但内容仍然是每次整体替换，而不是增量 patch。

**给你的建议**：

- 如果你用 Rust，ratatui 的即时模式几乎是默认选项，直接拥抱它——不要试图在上面
  再手撸一层"widget 树 + diff"，库层的 `Buffer` diff（见第 3 节）已经处理了性能问题。
- 如果你的应用有明显的"消息驱动"结构（网络请求、订阅、多个独立的异步来源），
  Elm 架构能帮你少踩很多"状态在哪改的"的坑，即使你不用 bubbletea，也可以在 ratatui
  里手动实现一个简化版（`zellij-workbench` 的 `mpsc` 后台刷新通道，本质上就是一个
  简化的 `Cmd → Msg` 通道，见第 6 节）。
- 如果你的应用有"面板 + 弹窗 + 多层导航"这种复杂度（像 lazygit），迟早需要一个
  持久化的视图对象 + 显式脏标记，纯函数式的"每帧重新构建一切"会因为状态量太大
  而变得笨重。

---

## 2. 事件循环：TUI 的心脏

不管选哪种范式，事件循环的骨架都长一个样：

```text
loop {
    读取/等待事件（键盘、鼠标、resize、定时器、后台任务完成通知）
    根据事件更新状态
    按需重新渲染
}
```

**`zellij-workbench` 的实现**（`src/tui.rs::draw_tui`）：

```rust
loop {
    apply_completed_refresh(&refresh_rx, ..., &search, view, server_filter.as_deref());

    if last_auto_refresh.elapsed() >= AUTO_REFRESH_INTERVAL && !auto_refresh_in_flight {
        spawn_auto_refresh(refresh_tx.clone());
        auto_refresh_in_flight = true;
        ...
    }

    terminal.draw(|frame| { /* 渲染整棵 UI 树 */ })?;

    if event::poll(Duration::from_millis(200))? {
        if let Event::Key(key) = event::read()? {
            match mode { /* 处理按键 */ }
        }
    }
}
```

三个关键设计点：

1. **`event::poll(Duration::from_millis(200))` 而不是 busy loop**：如果直接
   `event::read()` 阻塞式读取，你没法在等待键盘输入的同时检查"该不该自动刷新了"；
   如果用无超时的忙轮询 `loop { if let Ok(ev) = try_read() {...} }`，会把一个 CPU
   核心跑到 100%。`poll(timeout)` 是两者的折中：最多等 200ms，要么等到事件提前返回，
   要么超时后回到循环顶部检查"自动刷新"之类的周期性任务。200ms 对人类按键来说完全
   感知不到延迟，但足够低频到不浪费 CPU。
2. **渲染在事件处理之前**：每次循环先 `terminal.draw`，再等事件——这保证了"任何
   状态变化（哪怕是后台线程刚推过来的）都会在下一次循环里被画出来"，不需要额外的
   "是否 dirty" 判断（ratatui 自己在更底层做 diff，见第 3 节）。
3. **事件循环单线程，副作用去别的线程**：整个 `draw_tui` 函数只在一个线程里跑。
   任何可能阻塞的操作（扫描 zellij 会话、跑 git 命令）都不允许直接放进这个循环，
   否则一次扫描卡住 3 秒，整个 UI（包括按键响应）就会跟着卡 3 秒。这就引出第 6 节。

**lazygit/gocui 的实现**（`pkg/gocui/gui.go`）更进一步，是"两个 channel 喂一个
单线程事件循环"：

- `pollEvent()` goroutine 只做一件事：把 tcell 的终端事件塞进 `g.gEvents`（带缓冲的
  channel）。
- 任何后台 goroutine 想更新 UI，必须调用 `Gui.Update(f func(*Gui) error)` 或
  `Gui.UpdateContentOnly(f)`，本质是把一个闭包塞进 `g.userEvents` channel：

  ```go
  func (g *Gui) Update(f func(*Gui) error) {
      task := g.NewTask()
      select {
      case g.userEvents <- userEvent{f: f, task: task}:
      default:
          panic("gocui: userEvents channel full; refusing to block or reorder")
      }
  }
  ```

  注意这里**故意用非阻塞的 `select`/`default`**：如果 channel 满了就直接 panic，
  而不是阻塞等待或者退化成"直接同步执行"。原因很实际——从 UI 线程自己往自己发送
  事件如果阻塞会死锁；而"满了就退化成同步执行"看似安全，实际会破坏调用者依赖的
  "事件按顺序处理"的假设。宁可 panic 暴露 bug，也不要静默破坏顺序保证。

- 主循环 `processEvent()` 对这两个 channel 做一次 `select`，然后
  `processRemainingEvents()` 会非阻塞地把当前已经排队的事件全部处理掉，**再决定
  是否要渲染**——这是显式的"事件合并"：与其每个按键都触发一次完整渲染，不如把
  短时间内挤在一起的一批事件先处理完，只渲染一次。

**通用结论**：无论语言/框架，"单线程事件循环 + 所有跨线程通信走 channel/队列"
几乎是 TUI（乃至任何有状态交互式程序）的铁律。你可以选择让这个约束在类型系统里
体现（Elm 架构的 `Cmd`/`Msg`），也可以像 gocui 那样用运行时 panic 兜底，但"UI 状态
只能被拥有它的那个线程修改"这条线不能碰。

---

## 3. 渲染性能：即时模式不等于暴力全屏重画

即时模式最常见的误解是"每帧重画所有 widget = 每帧把所有字节都发给终端"。实际上
这是两件事，中间隔着一层 diff：

**ratatui 的 Buffer diff**：`Terminal::draw()` 把 widget 渲染进一个内存里的
`Buffer`（一个二维 cell 数组），frame 结束后并不是把整个 `Buffer` 转成转义序列发
给终端，而是用 `ratatui-core` 的 `BufferDiff` 迭代器，只把这一帧里**真正发生变化**
的 `(x, y, cell)` 发给后端（crossterm/termion/…）：

> `BufferDiff`：一个零分配的迭代器，比较两帧 buffer 的差异，只产出 `next` 里与
> `prev` 对应位置不同的 `(x, y, &Cell)`。

**gocui/tcell 这边是同一套思路，只是发生在更底层**：`View.draw()`
（`pkg/gocui/view.go`）把字符写进 tcell 的 `CellBuffer`；`CellBuffer.Dirty(x, y)`
比较这一帧和上一帧的 `currStr`/`lastStr`，`Screen.Show()` 触发的
`drawCell()` 会先判断 `if !t.cells.Dirty(x, y) { return }`，**没变化的格子根本不会
被写进要发给终端的字节流**。

**结论 1：你几乎不需要手写"脏矩形"逻辑**。ratatui/tcell 这类库已经在 cell 级别做
了 diff，这部分性能你白得。你需要关心的是下面两件事：

**结论 2：真正的性能陷阱在"逻辑重建"这一层，不在"物理发送"这一层**。即使物理发送
已经被 diff 过滤了，如果你在渲染闭包里为一个根本不在屏幕上的 1 万行列表格式化了
1 万个字符串，这个 CPU 开销已经发生了，diff 只是省了"发给终端"的那一小步。lazygit
对这一点处理得非常具体：

- **列表虚拟化**（`pkg/gui/context/list_context_trait.go::HandleRender`）：只有当
  `renderOnlyVisibleLines` 为真时，才会用视口的可见范围 `ViewPortYBounds()` 去调用
  `renderLines(startIdx, startIdx+length)`——**只格式化当前能看到的那几十行**，
  而不是整个提交历史/文件列表。代码注释写得很直白：「只渲染可见区域能为那些可能
  变得很长的列表节省大量内存」。
- **跳过整个布局重算的"仅内容"快路径**：`flushContentOnly()` 在这一批事件全部标记
  为 `contentOnly` 时，直接跳过昂贵的 `Layout()` 重算，只重画被标记 `tainted` 的
  view（以及和它们区域有重叠的 view）。lazygit 用它来做状态栏里的转圈动画——每
  100ms 跳一次的 spinner，没有理由触发一次完整的窗口布局重排。
- **昂贵计算的显式缓存**：commit 图里的 "pipe"（那些连接 commit 节点的竖线/斜线）
  计算量不小，`pkg/gui/presentation/commits.go` 用一个按
  `(commitHash, commitCount, divergence)` 做 key 的 `pipeSetCache` 存住结果；
  同理，把带颜色的字符串转成 ANSI 转义序列的结果也被 `rgbCache` 缓存；`git config`
  的读取结果被 `CachedGitConfig` 缓存，避免每次刷新都重新 fork 一个 `git config`
  子进程。
- **限流突发请求**：`pkg/tasks/tasks.go` 里有两个很小但很关键的常量：

  ```go
  const THROTTLE_TIME = time.Millisecond * 30
  const COMMAND_START_THRESHOLD = time.Millisecond * 10
  ```

  当用户快速上下翻动 commit 列表时，每次移动光标都可能触发一次 `git show`；如果
  上一个命令还没执行完就被取消了（用户又移动了），且系统看起来已经有点吃力
  （启动耗时超过 10ms、总耗时却不到 30ms），下一个命令会先 `sleep 30ms` 再启动——
  这是显式地避免"用户快速翻页 = 疯狂 fork 子进程"的资源踩踏。

**给你的检查清单**：

- 长列表一定要做视口窗口化（只格式化可见行），而不是依赖底层 diff 来"兜底"——
  diff 只能省发送成本，省不了你构建字符串/computing display value 的成本。
- 任何"同一份输入、短时间内会被重复计算"的东西（着色字符串、图状结构、外部命令
  结果）都值得考虑用一个简单的 `HashMap` 做 memoization。
- 高频但视觉上很轻的更新（进度条、spinner、状态文字）应该有一条不触发整体布局
  重算的快路径。
- 用户可以快速触发的操作（翻页、搜索输入）如果背后连着重量级命令，要考虑"取消
  上一个 + 限流下一个"，而不是无脑并发跑。

---

## 4. 布局系统：约束驱动，而不是像素计算

好的 TUI 布局系统都收敛到同一个思路：**声明式的约束/权重，而不是手算 x/y 坐标**，
这样终端 resize 时只需要重新求解约束，不需要改任何业务代码。

**ratatui 的 `Layout` + `Constraint`**，`zellij-workbench` 里的真实用法
（`src/tui.rs::draw_tui`）：

```rust
let shell = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
        Constraint::Length(3),  // 顶部状态栏：固定 3 行
        Constraint::Min(8),     // 中间主体：至少 8 行，其余空间都给它
        Constraint::Length(1),  // 底部快捷键提示：固定 1 行
    ])
    .split(frame.area());

let chunks = Layout::default()
    .direction(Direction::Horizontal)
    .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
    .split(shell[1]);
```

这里能看到典型的**嵌套切分**：先把整个终端竖切成"头/体/脚"三段（`Length`/`Min`/
`Length` 混用），再把中间那段横切成 52%/48% 两栏（列表 + 详情）。resize 之后
这段代码完全不用改——下一次 `frame.area()` 换了尺寸，`split` 会重新按约束求解。

**lazygit 的 `boxlayout`**（vendor 进来的 `lazycore/pkg/boxlayout`）走的是同一条
路，只是做成了一棵可递归嵌套的树：

```go
type Box struct {
    Direction Direction   // ROW 或 COLUMN
    Children  []*Box
    Window    string      // 叶子节点：这个 box 对应哪个窗口
    Size      int         // 静态大小（和 Weight 二选一）
    Weight    int         // 动态权重，按比例分配剩余空间
}
```

`ArrangeWindows` 的分配算法本质是 flexbox：**先满足所有指定了 `Size` 的子节点，
剩余空间再按 `Weight` 比例分给其余子节点**——这和 CSS 的 `flex-grow` 是同一个模型。
`pkg/gui/controllers/helpers/window_arrangement_helper.go::GetWindowDimensions`
每一帧都会用当前的屏幕宽高、聚焦窗口、屏幕模式（正常/半屏/全屏）重新构建整棵
`Box` 树，再调 `ArrangeWindows` 求解——**布局是每次都从头算的，不做增量 patch**，
因为这棵树本身很小，重算的开销可以忽略不计。这也解释了 lazygit 怎么做到 resize
零特殊处理：`flush()` 检测到终端尺寸变化后，直接重跑一遍 `Layout(g)`，新尺寸
自然被下一次布局计算吸收。

**两边共同的经验**：

1. **约束系统的复杂度应该和布局复杂度匹配，不需要一步到位上最复杂的方案**。
   `zellij-workbench` 一个两栏 + 头尾的静态布局，`Layout`/`Constraint` 足够；
   lazygit 有"正常/半屏/全屏"、"横屏/竖屏切换（`shouldUsePortraitMode`）"、
   "手风琴模式展开聚焦面板"等一堆动态规则，值得为此单独抽一个可递归的 `Box` 树。
2. **响应式不是"特殊处理 resize 事件"，而是"每次渲染都重新求解布局"的自然副产品**。
   两边都没有为 resize 写专门的分支逻辑——因为布局本来就是每帧算的，尺寸只是
   输入参数之一。
3. **布局与内容解耦**：无论是 `Layout::split` 返回的 `Rect` 数组，还是
   `ArrangeWindows` 返回的 `map[string]Dimensions`，都只回答"这块区域多大在哪"，
   不掺杂"这块区域里画什么"——后者始终是渲染阶段单独决定的。

---

## 5. 状态与导航：从一个枚举到一个上下文栈

`zellij-workbench` 的 TUI 目前只有两种"模式"：

```rust
enum InputMode {
    Normal,
    Search,
}
```

一个枚举 + `match` 分支，对于"列表浏览 + 搜索输入框"这种扁平交互完全够用，代码里
每个按键处理分支清清楚楚。

但如果你的应用需要"面板 A 里按某个键弹出一个菜单，菜单里再弹一个确认框，Esc 要能
一层层退回去，而且退回去之后原来那个面板的滚动位置/选中项要原样保留"——这种需求
一旦出现，扁平的枚举会迅速膨胀成一个"所有模式的笛卡尔积"式的巨型 `match`。

lazygit 选择了**显式的上下文栈**（`pkg/gui/context.go::ContextMgr`）：

```go
type ContextMgr struct {
    ContextStack []types.Context
    sync.RWMutex
    ...
}
```

每个面板/弹窗都实现 `types.Context` 接口，并声明自己的"种类"（`ContextKind`）：
`SIDE_CONTEXT`（侧边栏，如文件/分支/commit 列表）、`MAIN_CONTEXT`（主内容区，
同一时间只允许一个）、`TEMPORARY_POPUP`（用完即扔的弹窗，如确认框）、
`PERSISTENT_POPUP`（可以叠加的弹窗，如菜单）等等。压栈规则按种类走不同逻辑：

- 压入一个 `SIDE_CONTEXT` → **清空整个栈**只留它自己（侧边栏之间互斥，不允许嵌套）。
- 压入一个 `MAIN_CONTEXT` → 移除栈里其他的 `MAIN_CONTEXT`，但保留侧边栏等其他层
  （同一时间只有一个主视图，但它可以叠在侧边栏"之上"）。
- 其他情况（菜单、确认框……）→ 如果栈顶是一个"临时弹窗"，直接替换它而不是嵌套
  （代码注释坦诚地说这是因为临时弹窗复用同一个 view，理论上应该支持"逐层返回"，
  但目前的实现做不到，这是已知的取舍，不是假装完美）。

这个栈同时也是**键盘事件路由的唯一依据**：`ContextMgr.Activate` 激活一个 context
时会调用 `gocui.Gui.SetCurrentView(viewName)`，而 gocui 的 `matchView()`
（决定一个按键该由谁处理）只认 `g.currentView`。换句话说，"现在按键该发给谁"这个
问题，答案永远是"看上下文栈顶"，不需要额外维护一份"当前聚焦面板"的状态副本。

**什么时候该从枚举升级到栈**：

- 你的"模式"之间开始出现真正的**嵌套**关系（弹窗上面还能再弹弹窗）而不是简单的
  互斥切换 → 该上栈了。
- Esc/返回键的语义变成"退回上一层"而不是"回到某个固定的默认状态" → 栈能天然
  表达这个语义，枚举只能靠额外记录"上一个状态是什么"来模拟。
- 不同"模式"需要拥有独立的按键绑定表，而不是在一个大 `match` 里穷举所有模式的
  所有按键组合 → 每个 context 自己声明自己的 keybinding，注册时自然按栈顶路由。

**不要过早引入**：如果你的应用确实只有"浏览 + 搜索"这种扁平交互（`zellij-workbench`
现在就是这样），一个枚举远比一整套 context 抽象好维护、好读。上下文栈是给"确实
有嵌套导航需求"的应用准备的，不是 TUI 的标配起手式。

---

## 6. 异步与响应性：永远不要阻塞事件循环

第 2 节说了事件循环必须单线程，那耗时操作（网络、子进程、磁盘）怎么办？两边给出
的答案高度一致：**开一个线程/goroutine 去做，通过 channel 把结果送回来，事件循环
只负责非阻塞地"看看有没有新结果"**。

**`zellij-workbench` 的后台刷新**（`src/tui.rs`）：

```rust
let (refresh_tx, refresh_rx) = mpsc::channel();
...
fn spawn_auto_refresh(refresh_tx: Sender<RefreshResult>) {
    thread::spawn(move || {
        let result = refresh_index_report()
            .and_then(|summary| { /* 重新从 SQLite 读一遍 */ })
            .map_err(|err| format!("{err:#}"));
        let _ = refresh_tx.send(result);
    });
}
```

主循环每次迭代开头都会调用 `apply_completed_refresh`，它用 `refresh_rx.try_recv()`
（非阻塞）把已经跑完的后台任务结果取出来、合并进 UI 状态（同时通过
`selected_workspace_id`/`restore_selection` 保住用户当前选中的行，不会因为数据
刷新就把光标弹回列表顶部）。**扫描 zellij 会话、跑 git 命令这些可能耗时几百毫秒
到几秒的操作，全程都在另一个线程里跑，事件循环该 200ms 轮询一次按键还是照常轮询**，
不会因为后台在扫描就卡顿。这本质上就是一个手搓的、简化版的"`Cmd` 派发 + `Msg`
回收"（对应第 1 节说的 Elm 架构思路），只是没有上升成一个正式的类型系统契约。

**lazygit 的两层异步**：

1. **`Gui.OnWorker(f func(Task) error)`**——开一个 goroutine 跑 `f`，framework
   自动包一层 panic 恢复（避免一个后台任务崩溃直接带崩整个终端），并且用一个
   `TaskManager` 追踪"现在还有没有后台任务在跑"（用来判断该不该显示"正在处理"
   的 loading 指示）。
2. **`RefreshHelper.Refresh(options)`**——一次"刷新"可能同时涉及好几类数据
   （文件状态、分支、commit、stash……），每一类各自通过 `OnWorker` 并发跑，跑完
   之后**必须**通过 `OnUIThread(func() error {...})`（本质就是 `Gui.Update`）
   把结果写回 UI，绝不允许 worker goroutine 直接改 view。为了防止"文件列表还没
   刷新完，又来了一次刷新"互相踩踏，每一类数据各自有一把锁
   （`RefreshingFilesMutex`、`RefreshingBranchesMutex`……）。

**进一步的精细控制——`pkg/tasks/tasks.go` 里的 `ViewBufferManager`**：翻 commit
历史时每选中一个 commit 就要跑一次 `git show`，但输出可能很长。与其等命令完全
跑完再一次性塞进 view，不如边读边显示：`NewCmdTask` 为每个命令起了 4 个协作的
goroutine——一个扫 stdout 按行灌进 channel，一个在 200ms 内还没读到内容时先显示
"loading…"占位符，一个消费者把已到达的行写进 view 并触发局部重绘，还有一个专门
监听取消信号、负责在用户翻到下一个 commit 时优雅终止上一个还没跑完的进程。

**通用检查清单**：

- 任何可能阻塞超过一帧时间（大约 16-100ms 量级，取决于你要的流畅度）的操作，
  都不能直接放进事件循环所在的线程。
- 后台线程/goroutine 计算完的结果，必须通过 channel/队列交回事件循环所在的线程
  再去改状态——不要用 `Mutex<State>` 直接跨线程改共享状态然后指望渲染线程"自然
  看到"，这条路能走通但心智负担和潜在的 data race 排查成本都远高于显式的消息
  传递。
- 同一类刷新如果可能被高频触发（用户狂按刷新键、快速翻页），要么去重（"已经在
  跑就不重复启动"），要么限流（跑完等一小会儿再接受下一次），否则耗时任务会
  越堆越多。
- 长输出的命令考虑流式消费而不是"跑完再整体处理"，能显著改善"大仓库/长历史"下
  的可感知延迟。

---

## 7. 必要功能清单：一个"好用"的 TUI 需要什么

把前面几节的经验换成一份更"产品向"的清单，这些是从 `zellij-workbench` 和 lazygit
两边的实现里各自能找到具体证据的功能点：

- **快捷键要可发现，不能靠用户去记文档**。`zellij-workbench` 的底部状态栏
  （`footer()` / `controls_line()`）常驻展示当前模式下所有可用按键；lazygit
  做得更进一步，`OptionsMapMgr` 把当前 context 的按键渲染进底部选项栏，还有一个
  专门的 `?` 键弹出完整的可搜索键位菜单（`OptionsMenuAction`）。**最小可行版本
  就是一行"当前可用按键"的状态栏，这个投入产出比极高。**
- **搜索/过滤要支持结构化查询，不只是纯文本包含匹配**。`zellij-workbench` 的
  `SearchQuery::parse`（`src/tui.rs`）支持 `server:` `status:` `tag:` `git:`
  前缀语法叠加纯文本关键词，一行输入能表达"prod 服务器上、状态是 active、
  git 是 dirty 的工作区"这种复合条件。
- **异步操作要有可见的状态反馈**，哪怕只是一行文字。`zellij-workbench` 的
  `scan_status`（"Scan: idle" / "refreshing..." / "ok, N workspaces" /
  "failed (...)") ；lazygit 有转圈 spinner。用户需要知道"它在干活"还是"卡住了"。
- **默认操作要克制，危险操作要显式升级**。`zellij-workbench` 特意让
  `attach`（附着）在会话缺失时报错并提示"用 `recreate`"，而不是自动帮你静默建
  一个新会话——避免把一个你以为还在、其实已经没了的工作区不声不响地"复活"成
  一个空会话。lazygit 对 force push、丢弃修改这类操作会弹确认框。
- **后台刷新不能打断用户当前的操作状态**。`restore_selection`（`src/tui.rs`）
  在数据重新加载后，按 ID 而不是按下标去找回原来选中的那一行——如果只用最简单的
  "刷新后 selection 归零"，用户每 30 秒就会被强制弹回列表顶部一次，体验会很糟。
- **resize 要免特殊处理**。见第 4 节——只要布局是"每帧从当前终端尺寸重新求解"，
  你几乎不需要为 resize 单独写代码。
- **终端状态必须能在异常退出时被恢复**。这是本文写作过程中在这个仓库里发现的
  一个真实问题：`run_tui()` 原来的写法是

  ```rust
  enable_raw_mode()?;
  execute!(stdout, EnterAlternateScreen)?;
  let result = draw_tui(&mut terminal, workspaces); // 如果这里 panic……
  disable_raw_mode()?;                               // ……这两行就永远不会跑
  execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
  ```

  一旦 `draw_tui` 内部 panic（哪怕只是一个下标越界的低级 bug），`disable_raw_mode`
  和"离开备用屏幕"就不会执行，用户的终端会卡在 raw mode + 备用屏幕里，回车没反应、
  `Ctrl-C` 也可能失灵，非常糟糕的体验。修复方式是用
  `std::panic::catch_unwind` 把渲染循环包起来，恢复终端状态之后再把 panic
  重新抛出去：

  ```rust
  let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
      draw_tui(&mut terminal, workspaces)
  }));
  disable_raw_mode()?;
  execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
  match result {
      Ok(result) => result,
      Err(payload) => std::panic::resume_unwind(payload),
  }
  ```

  lazygit 这边同样有这类考量：`onWorkerAux`（`pkg/gocui/gui.go`）给每个后台
  goroutine 包了 panic recover，未恢复的 panic 会先调用 `Screen.Fini()`（tcell
  用来退出备用屏幕、恢复终端模式的清理函数）再继续 panic，**原则是一样的：任何
  可能 panic 的路径，只要它发生在"终端已经被切换到特殊模式"之后，就必须显式兜底
  恢复终端，不能依赖正常执行路径顺带做这件事**。这个教训几乎对所有直接操作终端
  raw mode 的语言/框架通用（Node 的 blessed/ink、Python 的 curses 都有等价的
  `finally`/`atexit` 模式）。

---

## 8. 案例对照表

| 维度 | zellij-workbench（ratatui） | lazygit（gocui/tcell） |
|---|---|---|
| 架构范式 | 即时模式：每帧重新构建 widget 树 | 持久 `View` 对象 + 脏标记局部重建 |
| 事件循环 | 单线程 `loop`，`event::poll(200ms)` | 单线程 `processEvent`，两个 channel（输入 / 用户回调）汇合 |
| 渲染 diff | `ratatui-core::BufferDiff`（cell 级） | tcell `CellBuffer.Dirty()`（cell 级）+ gocui 自己的 `tainted`/内容专用快路径（view 级） |
| 长列表优化 | 未做虚拟化（当前数据规模不需要） | `RenderOnlyVisibleLines`：只格式化可见窗口内的行 |
| 布局系统 | `Layout` + `Constraint`（Length/Percentage/Min） | 自研 `boxlayout`：`Size`/`Weight` 递归树，flexbox 式算法 |
| 导航/模式 | 扁平枚举 `InputMode { Normal, Search }` | `ContextMgr` 上下文栈，按 `ContextKind` 决定压栈/弹栈规则 |
| 异步 | `thread::spawn` + `mpsc::channel`，主循环非阻塞 `try_recv` | `OnWorker` goroutine + `OnUIThread`/`Update` 回传，按 scope 加锁避免刷新踩踏 |
| 限流/缓存 | 暂无（扫描频率低、数据量小，暂不需要） | commit 图缓存、颜色字符串缓存、git config 缓存、命令启动限流（30ms） |
| 快捷键发现 | 底部固定提示行 | 底部选项栏 + `?` 完整键位菜单，二者共享同一份 binding 数据 |
| 终端状态恢复 | `catch_unwind` 包裹渲染循环（本次修复） | 后台 goroutine panic recover + `Screen.Fini()` |

**怎么读这张表**：不是"lazygit 处处更强"，而是两边的复杂度投入和各自的问题规模
匹配——lazygit 要处理任意大小的真实 Git 仓库（可能几十万次 commit）、复杂的多面板
导航、多种屏幕模式，所以在虚拟化、缓存、限流、上下文栈上都做了真功夫；
`zellij-workbench` 现阶段的数据规模（几十到上百个工作区）和交互复杂度（列表 +
详情 + 搜索）用即时模式 + 扁平状态就完全够用。**先把架构复杂度控制在"刚好够用"，
等真的遇到"这个操作明显卡了"或者"这个模式管理已经乱了"的信号，再按本文的方向去
加虚拟化、缓存、上下文栈——不要在需求出现之前就把复杂度预支了。**

---

## 9. 延伸阅读

- [Ratatui：Rendering under the hood](https://ratatui.rs/concepts/rendering/under-the-hood/) ——
  官方文档对 `Buffer` diff 机制的详细说明。
- [Ratatui ARCHITECTURE.md](https://github.com/ratatui/ratatui/blob/main/ARCHITECTURE.md) ——
  0.30 之后的多 crate 拆分（`ratatui-core`/`ratatui-widgets`/各 backend）。
- [The Elm Architecture（bubbletea 版）](https://github.com/charmbracelet/bubbletea) ——
  Model-Update-View 模式在 Go 里的落地，附带官方 tutorial。
- [lazygit 源码](https://github.com/jesseduffield/lazygit)，尤其是
  `pkg/gocui/gui.go`（事件循环）、`pkg/gui/context.go`（上下文栈）、
  `pkg/tasks/tasks.go`（流式命令输出与限流）、
  `vendor/.../lazycore/pkg/boxlayout`（布局算法）。
- `zellij-workbench` 本仓库的 `src/tui.rs`——本文几乎每个 ratatui 相关的例子
  都直接摘自这个文件，建议对照阅读。

如果你准备从零写一个 TUI，建议的起步顺序是：先用即时模式 + 一个扁平的状态枚举
把主流程跑通（对应本文第 1、5 节的"起手式"），然后按第 2 节把事件循环和异步刷新
的骨架搭对，再根据实际遇到的性能/导航复杂度问题，参考第 3、4、5 节里 lazygit 的
做法逐步"加料"——而不是一开始就把上下文栈、虚拟化列表、限流这些为大规模场景准备
的机制全部搬进来。
