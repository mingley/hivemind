def allow(tokens: int, window_end: int, now: int) -> bool:
    # WIP: off-by-one — should be `now <= window_end`
    return now < window_end and tokens > 0
