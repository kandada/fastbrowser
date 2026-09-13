import asyncio

import fastbrowser as fb


def test_async_basic_flow():
    async def main():
        b = fb.AsyncFastBrowser()
        await b.init()
        assert b.is_initialized

        out = await b.open("https://example.com")
        assert out["title"] == "Example Page"

        snap = await b.snapshot()
        assert snap["title"] == "Example Page"

        url = await b.tool_call("get_current_url")
        assert url["url"] == "https://example.com"

        png = await b.screenshot_png()
        assert png[:8] == b"\x89PNG\r\n\x1a\n"

        b.shutdown()
        assert not b.is_initialized

    asyncio.run(main())


def test_async_concurrent_multi_tab():
    async def main():
        b = fb.AsyncFastBrowser()
        await b.init()

        t1 = (await b.open("https://example.com"))["tab"]
        t2 = (await b.open("https://example.com/login"))["tab"]

        # 并发读两个不同标签页的标题（真并发）
        r1, r2 = await asyncio.gather(
            b.tool_call("get_page_title", {"tab": t1}),
            b.tool_call("get_page_title", {"tab": t2}),
        )
        assert r1["title"] == "Example Page"
        assert r2["title"] == "Login"

        b.shutdown()

    asyncio.run(main())


def test_async_errors():
    async def main():
        b = fb.AsyncFastBrowser()
        # 未初始化 → await 后抛异常
        try:
            await b.open("https://example.com")
            assert False, "未初始化应抛异常"
        except RuntimeError:
            pass
        b.shutdown()

    asyncio.run(main())
