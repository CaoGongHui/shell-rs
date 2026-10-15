# Kubernetes

[返回使用手册](manual.md#kubernetes)

Kubernetes 是远程终端右侧栏的一个工具。集群来自 **SSH 主机**上的 kubeconfig，不是运行 ShellRS 的电脑。连接到能访问集群的管理主机后，在右侧选择 Kubernetes。

可在「设置 → 终端 → 侧栏功能」开启或关闭 Kubernetes，默认关闭（不是每台主机都有 kubectl），选择会跨启动保存。关闭立即隐藏入口、释放面板并停止列表、日志和性能刷新；再次开启后，从终端右侧重新打开即可使用。

内存的容量、可分配量、容器 requests / limits 和实时用量统一显示为 G（1024³ 字节），小于 0.01 G 的正值显示为 `<0.01 G`。概览时间及 kubectl 日志时间前缀统一显示为东八区（UTC+08:00）；YAML、应用日志正文和复制的原始日志保留原值，日志搜索匹配显示的时间。

Kubernetes 面板在首次选择时才创建。未使用时不创建其输入控件或订阅、不探测 kubectl、不查询集群。隐藏后停止等待中的列表查询；已发送的 SSH exec 命令仍可能执行至结束或超时（最多 10 秒），迟到结果不会应用。查询使用独立于终端 shell 的 exec 通道，失败只显示在工具内，不作为终端连接失败。

## 查询范围

- **集群（context）**：读取 `kubectl config current-context` 和 `kubectl config get-contexts -o name`，支持选择其他 context。所有 API 请求带 `--context`，不会执行 `use-context` 或改动 kubeconfig。
- **命名空间**：通过 kubectl 按 Kubernetes API 分页读取完整列表，名称排序。默认“全部命名空间”，查询跨命名空间的 API 路径；选择一个 namespace 后，查询限定到该命名空间的路径。
- **面板**：资源、节点两个标签，默认是资源（Pod），共享当前 context。节点展示容量、地址和就绪状态；节点、Namespace、PV、StorageClass 的查询不带命名空间参数。
- **资源类型**：Pod、Deployment（deploy）、StatefulSet（sts）、DaemonSet、ReplicaSet、Service（svc）、Ingress、Job、CronJob、PVC、Namespace、ConfigMap、Secret、ServiceAccount、PV、StorageClass、HPA、PDB、EndpointSlice。Secret 的列表与详情仅读取元数据，不读取 `data` / `stringData`。
- **搜索**：在当前类型和命名空间查询结果中，按名称、命名空间、状态、标签进行不区分大小写的过滤，不额外执行服务器命令。

集群和命名空间下拉框支持在本地搜索，筛选后按实际名称选择；查看节点等集群级资源时隐藏命名空间筛选，保留之前选择以便返回命名空间资源。

普通资源列表使用 Kubernetes API 的 limit / continue 逐页读取，默认每页 100 项，每页单独传输，首批结果到达即显示。若某页触发 SSH 1 MB 或解压 16 MB 上限，会自动缩小页大小后重读同一页，保留相同查询范围和分页令牌；连续页校验 resourceVersion 和令牌，避免混合快照或无限重试。Secret 和 ConfigMap 列表仅传元数据，完整 ConfigMap 数据只在详情读取。

结构化查询在远端有 gzip 与 base64 时使用快速压缩传输；缺少工具时自动使用普通输出。后台解压和解析，单次解压最多 16 MB，列表累计最多 64 MB / 100000 项。共享 SSH 单次传输仍限制为 1 MB，每条命令最多 10 秒，API 请求超时为 8 秒；单项资源压缩后仍过大时明确报错，不能把截断内容当作完整 JSON。

## 资源详情与 Pod

点击资源名称打开详情，重新读取该资源。概览与 Docker 共用可选择文本的键值行，按重点属性与基本信息分组，默认最多展示 8 项重点属性；标签和完整状态条件等次要信息通过“显示全部属性”展开。值可选中文字后用系统复制快捷键复制，右键菜单可复制完整字段；“复制概要”复制所有已解析字段。资源列表的右键菜单提供复制名称。完整配置放在独立的 YAML 标签页，使用文件编辑器同款 YAML 高亮组件，默认只读；编辑入口会切换到可编辑配置。

列表采用固定两行结构，名称、状态、摘要的槽位及行高保持一致。列表不自己刷新（同 Docker）：显示、切换终端、改变查询范围、在这里修改资源之后和点刷新时才读；本地搜索不产生远程请求。查询错误保留旧数据并等待用户主动刷新。

## 编辑、重启与删除

列表右侧的“资源操作”菜单和右键菜单，以及详情顶部的图标按钮，提供与 Docker 相同风格的操作入口。编辑、重启、删除都通过工作区的同一个 Kubernetes 动作处理器执行。

- **编辑 YAML**：支持 Deployment、StatefulSet、DaemonSet、Pod、ConfigMap、Service、Ingress、Job 等普通资源。编辑时去除 `status` 和 `managedFields`；转成 JSON 后要在约 25 KB 以内：命令经 SSH 作为一个参数交给登录 shell，Linux 限制单个参数 128 KiB，而脚本按 `\0NNN` 编码后约为原来的 5 倍。名称、命名空间、类型、API 版本、UID 和 resourceVersion 不可更改，也不能提交资源列表或多文档 YAML。Secret 仅读取元数据，不提供编辑入口。
- **保存修改**：按钮或 Ctrl+S / ⌘S 先解析并检查 YAML，再弹出包含集群、资源类型、命名空间、名称和影响的确认框。确认后才提交更新；取消不会写入。无效 YAML 不请求服务器；失败或冲突保留编辑内容。保存成功重新读取详情并刷新列表。
- **重启工作负载**：Deployment、StatefulSet、DaemonSet 按更新策略重启，使用与 `kubectl rollout restart` 相同的模板重启注解。已暂停的工作负载和 OnDelete 策略不能自动重启，页面说明需先恢复更新或逐个重建 Pod。已进入删除流程的资源禁用重启，避免向正在终止的对象提交无效重启请求。
- **重启 Pod**：通过删除当前 Pod 并由其工作负载控制器创建替代对象实现。独立 Pod、没有受支持控制器的 Pod、已结束的 Pod 不提供直接重启；确认框说明中断、临时数据丢失及 Job 任务可能重复执行的影响。ConfigMap 不提供重启操作。
- **删除**：删除前再次确认，命名空间、存储资源、Pod 使用各自的影响说明。请求只针对确认框中的单个对象，携带 UID 和资源版本前置条件，使用正常优雅删除和后台级联策略。

更新采用带版本的单对象 PUT；工作负载重启使用带 UID / resourceVersion `test` 的 JSON Patch；删除使用带这两个前置条件的 DeleteOptions。同名资源被重建或其他人修改配置时，请求失败后需刷新重试，不自动覆盖、不强制替换。没有确认票据的命令不能直接提交写操作。写请求发出后即使关闭详情，仍观察结果并显示通知；列表只在原 SSH 终端和 context 仍匹配时刷新。

## 性能

节点与 Pod 的详情提供“性能”标签页，直接复用系统监控的卡片、环形仪表、进度条、主题负载色与数值动画。进入此页才通过 `metrics.k8s.io/v1beta1` 读取当前节点或 Pod 的 CPU、内存用量，每次查询结束后间隔 10 秒刷新。节点比例以节点 capacity 为分母；Pod / 容器比例以实际观测到的容器 limits 为分母，任何容器缺少限制就只显示实际用量，不把 requests 或部分 limits 当成总量。

节点磁盘另通过 kubelet Summary API 的 `node.fs` 读取根文件系统已用 / 总量，每 30 秒低频查询。PVC / PV 的容量配置不会被当作真实磁盘用量。CPU、内存需要集群提供 Metrics API 并允许读取相应对象；磁盘需要 `get nodes/proxy` 权限。不可用时显示原因，已有读数保留，CPU / 内存查询退避到 60 秒重试。磁盘读取随这条串行采集循环执行，失败退避期间也降至 60 秒。

离开性能页、切换资源或关闭详情会取消采集，迟到回复按代次丢弃。请求串行执行，指标解析在后台，沿用现有请求超时、压缩和大小上限。

加载动画沿用 Docker 的 Spinner 和刷新按钮。Pod、工作负载、Job、PVC 状态使用主题色标签：就绪 / 成功为绿色，等待 / 副本未就绪为警示色，失败 / CrashLoopBackOff 等为错误色；同时保留状态文字与就绪数量。

右上角“关闭”、Esc 或点击窗口外侧均可关闭详情，随后焦点回到资源列表。查询尚未完成时也可以关闭；迟到的回复不会重新打开窗口。

Deployment、StatefulSet、DaemonSet、ReplicaSet、Job 和带选择器的 Service 会按 `spec.selector` 查询同命名空间内的候选 Pod，支持 `matchLabels` 和 `matchExpressions`。工作负载再按 ownerReferences 和 UID 验证控制链，Deployment 先读取实际归属的 ReplicaSet，避免把标签相同的其他工作负载列为关联 Pod。概览中的关联 Pod 列表不受关系图 160 个节点的显示上限影响。点击关联 Pod 后，详情页切换到该 Pod，再进入日志页。没有直接 Pod 选择器的资源（例如 CronJob 或 ExternalName Service）可回到 Pod 列表按名称或标签筛选。

## 资源关系图

详情中的「关系图」标签页使用固定的「上游 → 当前资源 → 下游」三列布局，只显示直接关系，每侧最多 3 个节点，更多节点通过前后分页查看。主控制链最多出现 7 个节点，配置引用区另显示至多 3 项，没有全量树、交叉连线或横向滚动画布。节点保留类型文字与分类色标；点击节点在同一窗口继续查看其上下游和详情，「返回上个资源」恢复上一个目标。

工作负载主图仅显示直接控制器和受控资源，例如 Deployment → ReplicaSet → Pod → 容器，逐层点击查看。配置与存储引用独立展示，每页最多 3 项；ConfigMap、Secret、ServiceAccount、PVC 均可点击。普通、init 和 ephemeral 容器分别显示自身环境变量与实际挂载的配置，不混入其他容器的引用。

控制关系以 [ownerReferences 和 UID](https://kubernetes.io/docs/concepts/overview/working-with-objects/owners-dependents/) 为准，不以相同标签推断归属。查询 ReplicaSet / Job 时不使用工作负载的 Pod 标签筛选它们的元数据，避免漏掉合法子对象。Service 选择器只匹配同命名空间 Pod；无选择器时仅沿 [EndpointSlice](https://kubernetes.io/docs/concepts/services-networking/endpoint-slices/) 发现后端。EndpointSlice targetRef 和 PV claimRef 同样校验 UID，避免连接到重建的同名对象。Secret 反向引用包含 Ingress TLS 和 ServiceAccount。

关系查询只补查当前对象需要的直接资源。配置引用从对象详情解析，不逐项查询配置内容。完整关系结果缓存 30 秒，最多 4 个目标，每份最多 4 MB / 512 项；UID、resourceVersion 改变或刷新后重查。关闭详情、切换目标或刷新会取消旧查询，拒绝迟到结果。Secret 详情仍仅读取元数据。

## 容器日志与检索

- 容器列表包括 `containers`、`initContainers` 和 `ephemeralContainers`。优先选择 Pod 的默认容器注解指定的容器，否则选择第一个普通容器。
- 最近行数：100、500、1000（默认）、5000、10000。
- 时间范围：5m、15m、1h（默认）、6h、24h。
- “上一次容器”对应 `kubectl logs --previous`，用于查看所选容器上一次终止前的日志。
- 每次读取最多 512 KB，带时间戳；切换容器、行数、时间范围或上一次容器选项时，清除旧范围的快照并重新读取。
- 检索区分大小写；默认按字面文本匹配，也可启用正则表达式。不合法的正则会显示错误。检索显示“匹配行数 / 已读取行数”，不会把搜索文本拼到 shell 命令里。
- “实时预览（2 秒）”在上一条查询结束后间隔 2 秒读取一次，不重叠请求。缓冲保存最新一次读取窗口，最多 512 KB / 10000 行，超限会提示；不会不断累积历史。当前时间范围和行数仍适用，GUI 预览不保证保留两次查询间的全部高流量日志。要连续跟踪可复制 `--follow` 命令到终端。
- 自动滚动可关闭以阅读旧行。关闭实时预览、离开日志页、关闭详情或切换容器 / 范围时，取消原刷新任务并拒绝迟到回复；返回日志页后可继续刷新。查询失败时停止实时预览并显示原因。“上一次容器”只查询快照。
- 日志高亮区分时间戳、DEBUG / INFO / WARN / ERROR / FATAL 等级别，以及 JSON 中的键、字符串、数字、布尔值。级别颜色使用主题状态色，JSON 使用编辑器主题语法色。虚拟列表只绘制可见行；高亮范围随内容缓存，未变化的回复不重建或刷新界面。
- 右键日志行可复制完整内容，也可通过工具栏复制当前检索结果。单行预览最多 16 KB，防止超长日志拖慢绘制，复制和检索仍使用缓冲区内的完整行。

## 生成命令

详情页的“生成命令”提供：查询状态与事件（describe）、查询 YAML、查询容器日志、实时跟踪日志、进入容器（`kubectl exec -it … -- /bin/sh`）。每个命令都包含当前 context、namespace 和资源名，Pod 命令还包含所选容器；日志命令沿用当前日志范围选项。复制后可在这台 SSH 主机的终端执行，容器需要有 `/bin/sh` 才能使用该进入命令。

## 权限与失败处理

使用 kubeconfig 中现有的身份和 RBAC 权限。写操作另外需要相应资源的 `update`、`patch` 或 `delete` 权限。需要集群级 `list namespaces`；浏览需要相应资源的 `list`，详情需要 `get`，关联 Pod 需要 `list pods`，日志需要 `get pods/log`。复制 exec 命令不请求集群权限；用户执行时还需要 `create pods/exec`。

浏览和日志仅读取；编辑、重启、删除经二次确认后才修改集群，不保存 kubeconfig、token、客户端证书。namespace 查询被拒绝时，显示实际的 Forbidden 错误并保留 context 选择；不会把没有权限的 namespace 当成不存在。kubectl 未安装、context 无效、API 不可达、SSH 断开或数据被截断也会显示原因，可以修复后点击刷新。

## 开发与测试

`src/kubernetes/` 独立于 `workspace`：`commands.rs` 负责命令、退出状态封装；`model.rs` 负责纯解析、摘要、状态颜色语义、Pod 选择器；`panel.rs` 持有查询范围和列表；`details.rs` 持有固定查询目标的资源详情和日志；`logs.rs` 负责有界窗口和线性日志分词，`metrics.rs` 解析指标与容量，`mutation.rs` 负责 YAML 校验、对象身份、版本校验与写操作命令；`presentation.rs` 复用主题标签、Spinner、复制按钮。指标展示与 Docker 的字段行统一放在 `shared/`。所有用户命令经 `app::KubernetesAction` 路由到 `Workspace::on_kubernetes_action`，字段与日志复制也走此处理器。

使用共享 `ExecTarget` 在远程终端自己的 SSH 连接上执行，不建立第二次登录。stderr 与退出状态经过封装，因为共享 Exec API 只返回 stdout。实际执行脚本编码为 POSIX `printf %b` 支持的八进制字符串，避免登录 shell 是 fish/csh 时处理嵌套引号或历史 `!`；生成给人的命令仍是可读的 kubectl 命令。所有来自 kubeconfig、API 的参数都有 shell 引用。

异步结果按代次验证，改变主机或查询范围会取消等待并拒绝旧结果。对话框绑定打开时的 SSH 连接、context、namespace 和资源；关闭对话框后不保留实体。后台回复由共享终端的 UI 轮询处理，不由工作线程唤醒 GPUI。

验证命令：

```sh
cargo test --lib kubernetes::
cargo test --test ui kubernetes::
cargo test --lib i18n::
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

UI 测试在真实 Workspace 中点击工具、操作 namespace 与资源类型下拉框、打开详情、导航关联 Pod、切换容器、查询上一次容器日志、检索和复制命令。SSH 回复完全注入，不读取开发者 kubeconfig，不连接真实集群，不访问钥匙串。Unix 单元测试还通过项目的 `testing::sh` 验证实际 shell 执行、stderr 和退出状态。
