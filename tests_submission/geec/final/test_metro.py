from __future__ import annotations

from pathlib import Path
from typing import Callable, Optional, Any
import sys
from collections import deque
import csv
import pytest

# --- 测试配置 ---
FILE_NAME = "final.py"

FUNC_LOAD = "load_data"
FUNC_PARSE = "parse_input_query"
FUNC_GET_ID = "get_station_id"
FUNC_NEI = "get_neighbors"
FUNC_PATH = "find_path"
FUNC_FMT = "format_path"


# -----------------------
# fixtures: 拉取学生函数
# -----------------------

@pytest.fixture(scope="function")
def f_load(get_function) -> Callable[[Path], Any]:
	return get_function(FUNC_LOAD, FILE_NAME)


@pytest.fixture(scope="function")
def f_parse(get_function) -> Callable[[str], tuple[str, str]]:
	return get_function(FUNC_PARSE, FILE_NAME)


@pytest.fixture(scope="function")
def f_get_id(get_function) -> Callable[[Any, str, str], Optional[int]]:
	return get_function(FUNC_GET_ID, FILE_NAME)


@pytest.fixture(scope="function")
def f_neighbors(get_function) -> Callable[[Any, int], list[int]]:
	return get_function(FUNC_NEI, FILE_NAME)


@pytest.fixture(scope="function")
def f_find_path(get_function) -> Callable[[Any, int, int, set[int]], list[int]]:
	return get_function(FUNC_PATH, FILE_NAME)


@pytest.fixture(scope="function")
def f_format(get_function) -> Callable[[Any, list[int]], str]:
	return get_function(FUNC_FMT, FILE_NAME)


# -----------------------
# helpers：尽量鸭子类型
# -----------------------

@pytest.fixture()
def data_file(tmp_path: Path) -> Path:
	"""
	生成一个最小网络：
	  18号线: A(1) - B(2) - 国权路(3)   且 3 换乘到 10号线 的 国权路(5)
	  10号线: C(4) - 国权路(5) - D(6)   且 5 换乘回 3

	能稳定测试：
	  - 同线 prev/next 构建
	  - 换乘字段解析（"3"/"3/7" 这种格式）
	  - DFS find_path + visited 防环
	  - format_path 检测线路变化输出“换乘”
	"""
	p = tmp_path / "线路.csv"
	rows = [
		["站点ID", "线路名", "站名", "可换乘站点ID"],
		[1, "18号线", "A站", ""],
		[2, "18号线", "B站", ""],
		[3, "18号线", "国权路", "5"],  # 换乘到 5
		[4, "10号线", "C站", ""],
		[5, "10号线", "国权路", "3"],  # 换乘回 3
		[6, "10号线", "D站", ""],
	]
	
	# 用 utf-8-sig 更兼容（不少同学会用 excel/或按样例读）
	with p.open("w", encoding="utf-8-sig", newline="") as f:
		w = csv.writer(f)
		w.writerows(rows)
	return p




def _station_map(data: Any) -> dict[int, Any]:
	"""
	兼容:
	  - data.stations / data.station_dict / data.id_to_station (dict)
	  - data 本身就是 dict[int, Station]
	"""
	if isinstance(data, dict):
		return data
	for attr in ("stations", "station_dict", "id_to_station", "nodes"):
		if hasattr(data, attr):
			m = getattr(data, attr)
			if isinstance(m, dict):
				return m
	raise AssertionError(
		"无法从 SubwaySystem 中取到 stations 映射。建议在 SubwaySystem 上提供 stations: dict[int, Station]"
	)


def _get(obj: Any, *names: str, default=None):
	for n in names:
		if hasattr(obj, n):
			return getattr(obj, n)
		if isinstance(obj, dict) and n in obj:
			return obj[n]
	return default


def _line(st: Any) -> str:
	return _get(st, "line_name", "line", default="")


def _name(st: Any) -> str:
	return _get(st, "station_name", "name", default="")


def _prev(st: Any):
	return _get(st, "prev_id", "prev", default=None)


def _next(st: Any):
	return _get(st, "next_id", "next", default=None)


def _transfers(st: Any):
	return _get(st, "transfer_ids", "transfers", "transfer", default=None)


def _bfs_far_node(get_neighbors: Callable[[Any, int], list[int]], data: Any, start: int, min_dist: int = 4) -> int:
	q = deque([(start, 0)])
	seen = {start}
	best, best_d = start, 0
	while q:
		u, d = q.popleft()
		if d > best_d:
			best, best_d = u, d
		for v in get_neighbors(data, u):
			if v not in seen:
				seen.add(v)
				q.append((v, d + 1))
	if best_d < min_dist:
		pytest.skip(f"图太小/可达距离不足：从 {start} 出发最大距离={best_d}")
	return best


# -----------------------
# Module B: parse_input_query
# -----------------------

@pytest.mark.parametrize("q, expected", [
	("1号线,人民广场", ("1号线", "人民广场")),
	(" 1号线 , 人民广场 ", ("1号线", "人民广场")),
	("1号线，人民广场", ("1号线", "人民广场")),  # 全角逗号
	("1号线， 人民广场", ("1号线", "人民广场")),
])
def test_parse_input_query(f_parse, q, expected):
	assert f_parse(q) == expected


# -----------------------
# Module A: load_data 基本结构 + prev/next/transfer 性质
# -----------------------

def test_load_data_builds_bidirectional_prev_next(f_load, data_file):
	data = f_load(data_file)
	smap = _station_map(data)
	assert len(smap) > 0
	
	# 若字段存在，就做强一致性校验
	# A.next=B => B.prev=A；A.prev=B => B.next=A
	for sid, st in list(smap.items())[:500]:  # 防止数据太大跑太久
		nxt = _next(st)
		prv = _prev(st)
		
		if nxt is not None:
			assert nxt in smap, f"next_id 指向不存在站点: {sid}->{nxt}"
			assert _prev(smap[nxt]) == sid, f"next/prev 不一致: {sid}.next={nxt}, 但 {nxt}.prev={_prev(smap[nxt])}"
		
		if prv is not None:
			assert prv in smap, f"prev_id 指向不存在站点: {sid}->{prv}"
			assert _next(smap[prv]) == sid, f"prev/next 不一致: {sid}.prev={prv}, 但 {prv}.next={_next(smap[prv])}"


def test_load_data_parses_transfer_ids_as_ints(f_load, data_file):
	data = f_load(data_file)
	smap = _station_map(data)
	
	for sid, st in list(smap.items())[:500]:
		tr = _transfers(st)
		if tr is None:
			continue
		assert hasattr(tr, "__iter__"), f"transfer_ids 应可迭代: station {sid}"
		for x in tr:
			assert isinstance(x, int), f"transfer_ids 元素必须是 int: station {sid}, got {type(x)}"
			assert x in smap, f"transfer_ids 指向不存在站点: {sid} transfer->{x}"
			assert x != sid, f"transfer_ids 不应包含自身: {sid}"


# -----------------------
# Module B: get_station_id
# -----------------------

def test_get_station_id_roundtrip(f_load, f_get_id, data_file):
	data = f_load(data_file)
	smap = _station_map(data)
	sid, st = next(iter(smap.items()))
	
	line = _line(st)
	name = _name(st)
	assert line and name, "站点缺少 line_name / station_name，无法测试 get_station_id"
	
	got = f_get_id(data, line, name)
	assert got == sid


def test_get_station_id_missing_returns_none(f_load, f_get_id, data_file):
	data = f_load(data_file)
	assert f_get_id(data, "不存在线路", "不存在站点") is None


# -----------------------
# Module C: get_neighbors / find_path
# -----------------------

def test_get_neighbors_includes_prev_next_transfer_when_present(f_load, f_neighbors, data_file):
	data = f_load(data_file)
	smap = _station_map(data)
	
	for sid, st in list(smap.items())[:300]:
		nb = set(f_neighbors(data, sid))
		
		prv = _prev(st)
		nxt = _next(st)
		tr = _transfers(st)
		
		if prv is not None:
			assert prv in nb, f"neighbors 未包含 prev_id: {sid}->{prv}"
		if nxt is not None:
			assert nxt in nb, f"neighbors 未包含 next_id: {sid}->{nxt}"
		if tr is not None:
			for x in tr:
				assert x in nb, f"neighbors 未包含 transfer: {sid} transfer->{x}"


def test_find_path_returns_valid_walk(f_load, f_neighbors, f_find_path, data_file):
	data = f_load(data_file)
	smap = _station_map(data)
	start = next(iter(smap.keys()))
	end = _bfs_far_node(f_neighbors, data, start, min_dist=4)
	
	path = f_find_path(data, start, end, visited=set())
	assert isinstance(path, list)
	assert path, "BFS 已确认可达，但 find_path 未返回路径"
	assert path[0] == start and path[-1] == end
	assert len(path) == len(set(path)), "路径含重复节点：visited 可能未正确使用"
	
	for a, b in zip(path, path[1:]):
		assert b in set(f_neighbors(data, a)), f"非法路径边：{a}->{b} 不是邻居"


def test_find_path_uses_visited_to_avoid_infinite_recursion(monkeypatch, f_find_path):
	"""
	关键“防爆栈”测试：构造 1<->2 的环，终点=3 不可达。
	期望：不抛 RecursionError，返回 [] 或 None。
	"""
	# 拿到学生模块对象（通过函数的 __module__）
	mod = sys.modules[f_find_path.__module__]
	
	def fake_neighbors(_data, cur: int) -> list[int]:
		if cur == 1:
			return [2]
		if cur == 2:
			return [1]
		return []
	
	monkeypatch.setattr(mod, "get_neighbors", fake_neighbors)
	
	try:
		out = f_find_path(None, 1, 3, visited=set())
	except RecursionError:
		pytest.fail("出现 RecursionError：visited 可能未用于防止无限递归")
	
	assert out == [] or out is None


# -----------------------
# Module D: format_path
# -----------------------

def test_format_path_inserts_transfer_marker_on_line_change(f_load, f_neighbors, f_find_path, f_format, data_file):
	data = f_load(data_file)
	smap = _station_map(data)
	
	start = next(iter(smap.keys()))
	end = _bfs_far_node(f_neighbors, data, start, min_dist=4)
	path = f_find_path(data, start, end, visited=set())
	if not path or len(path) < 2:
		pytest.skip("找不到可用路径测试 format_path")
	
	# 找到相邻站点线路名变化的位置
	pair = None
	for a, b in zip(path, path[1:]):
		la, lb = _line(smap[a]), _line(smap[b])
		if la and lb and la != lb:
			pair = (a, b)
			break
	
	if pair is None:
		pytest.skip("未找到线路变化的相邻站（数据或构建方式导致）")
	
	out = f_format(data, [pair[0], pair[1]])
	assert isinstance(out, str)
	assert "换乘" in out, f"线路变化时应出现“换乘”，但输出为：{out}"
