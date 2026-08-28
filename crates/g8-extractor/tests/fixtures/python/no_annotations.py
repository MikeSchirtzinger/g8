# NO-detect fixture: Python file with no G8 annotations.

# A regular Python comment.
# @some_other_decorator — not G8

def regular_function(x: int) -> int:
    return x + 1


class RegularClass:
    """A regular class."""
    pass
