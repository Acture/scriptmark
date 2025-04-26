import copy
import sys

trace_output = []
trace_sequence = 0
exclude_names = set(["trace_output", "trace_func", "sys", "exclude_names", "is_basic_type_or_container", "trace_sequence"])


def is_basic_type_or_container(v, depth=0, max_depth=3):
	# 防止递归太深
	if depth > max_depth:
		return False

	# 基本类型判断
	if isinstance(v, (int, float, str, bool, type(None))):
		return True

	# 容器类型判断
	if isinstance(v, (list, tuple)):
		return all(is_basic_type_or_container(x, depth + 1, max_depth) for x in v)

	if isinstance(v, dict):
		return (all(isinstance(k, (str, int)) for k in v.keys()) and  # 键必须是字符串或整数
				all(is_basic_type_or_container(val, depth + 1, max_depth) for val in v.values()))

	if isinstance(v, set):
		return all(is_basic_type_or_container(x, depth + 1, max_depth) for x in v)

	# 排除模块和其他复杂类型
	return False


def safe_copy(v):
	try:
		return copy.deepcopy(v)
	except Exception:
		return v  # fallback


def trace_func(frame, event, arg):
	global trace_sequence, trace_output
	try:
		if frame.f_code.co_filename == "$filename":
			lineno = frame.f_lineno
			filtered_vars = [(trace_sequence, lineno, k, safe_copy(v)) for k, v in frame.f_locals.items() if not k.startswith('_') and k not in exclude_names and is_basic_type_or_container(v)]
			trace_output.extend(filtered_vars)
			trace_sequence += 1
	except Exception as e:
		trace_output.append((trace_sequence, None, "Trace error", str(e)))
		trace_sequence += 1

	return trace_func


sys.settrace(trace_func)
try:
	code = compile(
		'''
		$code
		''',
		filename="$filename",
		mode="exec"
	)
	exec(code)
finally:
	sys.settrace(None)
