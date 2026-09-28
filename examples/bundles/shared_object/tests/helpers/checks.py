"""Teacher checks for the account scenario.

A checker is called as fn(result, expected, **names): any further parameter is filled
from the names in scope — here `acct`, the object scenario setup built. Return a bool or
(bool, message); raise AssertionError to reject a malformed answer. Anything else it
raises means the checker could not decide, which is the teacher's to fix.
"""


def is_history_of(result, expected, acct):
    assert isinstance(result, list), f"history should be a list, got {type(result).__name__}"
    if not result:
        return False, "history is empty"
    return result[-1] == acct.balance, f"history ends at {result[-1]}, balance is {acct.balance}"
