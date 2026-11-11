import os
import time

import pytest


@pytest.fixture(autouse=True)
def _force_utc_timezone():
    """Keep datetime assertions stable across developer and CI TZ settings."""
    os.environ["TZ"] = "UTC"
    if hasattr(time, "tzset"):
        time.tzset()
    yield
