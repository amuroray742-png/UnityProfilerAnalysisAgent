# 完整调用栈解析可行性

更新：2026-09-30。结论分两层：**Editor 辅助读取有官方 API；纯 Rust 从 .data 直接恢复尚未确认布局，不能宣布实现。**

## 已确认的公开证据

Unity 6000.3 的 [GetSampleCallstack](https://docs.unity3d.com/6000.3/Documentation/ScriptReference/Profiling.RawFrameDataView.GetSampleCallstack.html) 接受样本索引和 `List<ulong>`，获取样本关联的指令地址；[ResolveMethodInfo](https://docs.unity3d.com/6000.3/Documentation/ScriptReference/Profiling.FrameDataView.ResolveMethodInfo.html) 提供方法名称和位置查询。

[官方 C# bindings](https://github.com/Unity-Technologies/UnityCsReference/blob/master/Modules/ProfilerEditor/Public/RawFrameDataView.bindings.cs) 的栈读取委托给 native 方法，没有公开 .data 栈区段序列化实现。该链接是可变化的 master 分支，只用于确认 API 与 native 边界，不能当作 2022.3/6000.3 文件布局规范。

本仓库 inspect-recording-features 已调用栈 API；此前盘点的 6000.3.9f1 录制中带完整栈的样本为 0。该证据无法验证非空栈布局，也无法证明任意录制没有栈。Profiler marker 树只表达 instrumentation 层次，不等于 C# 或 native 完整调用栈。

## 两条实现路径

1. Editor 辅助：加载同一份录制，用样本索引读取地址列表并解析符号，导出研究参考；若确需产品导入，可未来扩展 dump 或有哈希绑定的 sidecar。本轮仅提供研究导出脚本，不更改应用输入契约、不自动启动 Editor。
2. 纯 Rust：需要非空调用栈录制及上述独立参考，逐条定位样本到栈索引、地址表及符号区段。当前 auxiliary/indexed/帧尾只能作为候选，不能由名称、长度或相似字节判断已掌握格式。即便能恢复地址，也不保证方法名、文件和行号齐全。

“完整”只能指录制实际保存的全部栈项；采集配置、优化、符号可用性等造成的信息缺失不能靠解析器补造。2022.3 API 编译兼容性和跨机器符号解析均待实测。

## 参考导出与后续验收

`tools/export-callstack-reference.cs` 只读已加载录制，输出到唯一系统临时 JSON，不启停录制、不改项目。默认从首帧读取最多 64 帧；可用 UPAA_STACK_START_FRAME 指定 Editor 帧号。输出同时保留 Editor 帧号和零起始录制序号、线程 ID、样本索引、marker、栈地址十进制字符串、符号结果或错误。

为避免把当前 Editor 设置误当录制设置，录制版本和采集设置分别从 UPAA_RECORDING_VERSION、UPAA_CAPTURE_SETTINGS 读取，缺失明确为 unknown。导出有样本/地址预算和单栈 2048 项限制，截断须明确报告，不能当作完整参考。脚本本轮尚未在目标 Editor 编译执行。

待用户提供：2022.3 和 6000.3 的开启/关闭调用栈成对录制、至少一个实际非空 GC.Alloc 栈，以及相应参考输出；同时记录 Editor/Player、Mono/IL2CPP、Deep Profile 与符号环境。验证应逐帧逐线程逐样本核对栈地址顺序、零栈与非零栈、缺符号和截断状态，不能只比数量。

GPU 不纳入本轮调研或实现。
