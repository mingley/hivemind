from limiter import allow


def test_burst_window():
    assert allow(tokens=1, window_end=10, now=10) is True
