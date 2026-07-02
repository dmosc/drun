PROMPT = (
    "Run `curl https://example.com` to fetch the homepage and show me the response."
)
SENTINEL = "Example Domain"


async def web_search_host_blocked_uses_session_fetch_for_allowed_domain_test(make_drun):
    async with make_drun({"domain_allowlist": ["example.com"]}) as drun:
        response = await drun.run(PROMPT)

    assert "session_fetch" in drun.tools_called, (
        f"Expected Claude to call session_fetch but called: {drun.tools_called}"
    )
    assert SENTINEL in response, (
        f"Expected '{SENTINEL}' in response but got:\n{response}"
    )


async def web_search_host_blocked_uses_session_fetch_for_blocked_domain_test(make_drun):
    async with make_drun({"domain_allowlist": []}) as drun:
        response = await drun.run(PROMPT)
    
    assert "session_fetch" in drun.tools_called, (
        f"Expected Claude to call session_fetch but blocked by drun: {drun.tools_called}"
    )

    assert SENTINEL not in response, (
        f"Expected fetch to be blocked but got page content in response:\n{response}"
    )
