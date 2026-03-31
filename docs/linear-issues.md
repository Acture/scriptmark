# ScriptMark Linear Issues

> Project: **SCR** (ScriptMark)
> Notion: https://www.notion.so/31f72dda0deb8192a5b0f6cf22c6f538

## P0 — MVP

### SCR-1: CLI — clap grade/run/summarize 命令
- `scriptmark grade submissions/ -t tests/ -o output/`
- `scriptmark run` (只执行不汇总)
- `scriptmark summarize` (只分析已有结果)
- comfy-table 表格 + owo-colors 彩色输出
- **Blocked by**: 无
- **Status**: Todo

### SCR-2: Fixtures — value + function 来源 + $ref 引用
- TestSpec 加 `fixtures: Vec<Fixture>` 字段
- Fixture 支持 `value` (静态值) 和 `function` (调学生函数)
- args 中 `$id` 替换为 fixture 结果
- fixture 失败则依赖它的 case 标记 Error
- **Blocked by**: 无
- **Status**: Todo

### SCR-3: Spec model 更新
- TestCase 加 `id: Option<String>`
- TestCase 加 `context: Option<Value>`
- TestSpec 加 `fixtures: Vec<Fixture>`
- **Blocked by**: 无
- **Status**: Todo

## P1 — 实用功能

### SCR-4: Canvas API — roster pull
- `scriptmark roster pull --canvas`
- 从 Canvas LMS 拉取课程花名册
- course.toml 中配置 [canvas] section
- CANVAS_TOKEN 环境变量
- **Blocked by**: SCR-1

### SCR-5: Canvas API — grades push
- `scriptmark grades push --canvas`
- 上传分数到 Canvas assignment
- **Blocked by**: SCR-4

### SCR-6: Rhai checker
- `check = { rhai = "result.len() > 0" }`
- Rhai 内联表达式验证
- **Blocked by**: 无

### SCR-7: Python script checker
- `check = { python = "verifiers/check.py" }`
- 统一 JSON stdin/stdout 协议
- **Blocked by**: 无

### SCR-8: Fixture file source
- `file = "generators/make_data.py"`
- 教师脚本生成测试数据
- **Blocked by**: SCR-2

### SCR-9: JSON/CSV archive
- 结果归档到 JSON 或 CSV
- **Blocked by**: SCR-1

## P2 — 高级功能

### SCR-10: Parametrize — 生成器 + oracle
- `[cases.parametrize]` section
- 内置生成器: int, float, str, bool, choice, list
- Oracle: reference (教师参考实现), rhai, check, python
- seed 支持可复现
- **Blocked by**: SCR-6, SCR-7

### SCR-11: CompiledExecutor — C++/Java
- compile + run + IO 比较
- `compile = "g++ -o {output} {source}"`
- **Blocked by**: SCR-1

### SCR-12: Exec checker
- `check = { exec = "verifiers/check" }`
- 任意可执行文件验证
- **Blocked by**: 无

## P3 — 分发 + 生态

### SCR-13: PyO3 Python bindings
- maturin 构建
- pip install scriptmark
- **Blocked by**: SCR-1

### SCR-14: WASM checker
- `check = { wasm = "verifiers/check.wasm" }`
- wasmtime 加载执行
- **Blocked by**: SCR-12

### SCR-15: CI workflow
- cargo test + clippy + maturin wheel + 多平台
- **Blocked by**: SCR-13

## Done (已完成)

### SCR-0: Cargo workspace + core models + PythonExecutor + orchestrator
- 4 crate workspace
- Core: models, spec_loader, discovery, grading, roster
- Runner: 6 内置 checker, PythonExecutor, orchestrator
- 21 tests passing
- **Status**: Done
