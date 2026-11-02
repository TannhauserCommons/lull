# /// script
# requires-python = ">=3.11"
# dependencies = ["playwright==1.64.1"]
# ///
"""Renders lull's Discord pictures from the SVGs next to this file: badge.png and badge-paused.png
(512 px), cover.png (1024 px). Discord fetches them from this folder on GitHub (discord.rs).

    uv run app/discord/render.py

Needs Playwright's Chromium (`uv run --with playwright==1.64.1 playwright install chromium` once).
"""
import asyncio
import pathlib

from playwright.async_api import async_playwright

HERE = pathlib.Path(__file__).resolve().parent
PICTURES = {"badge": 512, "badge-paused": 512, "cover": 1024}


async def main() -> None:
    async with async_playwright() as p:
        browser = await p.chromium.launch()
        for name, size in PICTURES.items():
            page = await browser.new_page(viewport={"width": size, "height": size})
            await page.goto((HERE / f"{name}.svg").as_uri())
            await page.wait_for_load_state("networkidle")
            await page.screenshot(path=str(HERE / f"{name}.png"), omit_background=True)
            await page.close()
        await browser.close()


asyncio.run(main())
