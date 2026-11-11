from typing import NewType

UserId = NewType("UserId", str)


class User:
    def __init__(self, tenant: str, id: UserId):
        self.tenant = tenant
        self.id = id


def fetch_user(tenant: str, id: UserId) -> User:
    return User(tenant, id)
