def test_smismember(redis_client):
    # Requires Redis 7+: SMISMEMBER
    redis_client.sadd("acl", "alice")
    assert redis_client.execute_command("SMISMEMBER", "acl", "alice") == [1]
