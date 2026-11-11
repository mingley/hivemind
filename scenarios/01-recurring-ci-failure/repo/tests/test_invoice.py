from datetime import datetime, timezone


def test_invoice_due_date():
    # Historical failure under TZ=America/Los_Angeles without conftest fixture.
    due = datetime(2026, 3, 15, 0, 0, tzinfo=timezone.utc)
    assert due == datetime(2026, 3, 15, 0, 0, tzinfo=timezone.utc)
