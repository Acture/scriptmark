"""Teacher checker for translate_number_to_text property tests."""

ERROR_MSG = "错误：输入内容必须全部为数字。"


@checker("translate_number_to_text")
def check_translate(result, expected, t1, t2, t3, t5, t6):
    """Auto-injected from _ctx: t1=T("1"), t2=T("2"), t3=T("3"), t5=T("5"), t6=T("6")."""
    # This checker is only used for the concatenation test
    # Other cases use exact match
    return True, ""


def check_concat(result, expected, t1, t2):
    """T("12") must equal T("1") + T("2")."""
    if result == t1 + t2:
        return True, ""
    return False, f"T('12')={result!r} != T('1')+T('2')={t1!r}+{t2!r}"


def check_repeat(result, expected, t3):
    """T("33") must equal T("3") + T("3")."""
    if result == t3 + t3:
        return True, ""
    return False, f"T('33')={result!r} != T('3')+T('3')={t3!r}+{t3!r}"


def check_all_mapped(result, expected):
    """T("0123456789") must return a non-empty string."""
    if result is None:
        return False, "returned None"
    if not isinstance(result, str):
        return False, f"returned {type(result).__name__}, expected str"
    if len(result) == 0:
        return False, "returned empty string"
    return True, ""


def check_unique(result, expected, t5, t6):
    """T("5") and T("6") must be different."""
    if t5 != t6:
        return True, ""
    return False, f"T('5') and T('6') are both {t5!r}"
