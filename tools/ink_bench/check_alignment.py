"""Check whether a captured ink-bench stroke covers its input endpoints.

Usage: python3 check_alignment.py screenshot.png x0 y0 x1 y1
The screenshot must come from a clean app session after one ADB stylus swipe.
"""

import argparse

from PIL import Image


def is_ink(pixel: tuple[int, ...]) -> bool:
    red, green, blue = pixel[:3]
    return red >= 200 and 100 <= green <= 220 and blue <= 100


def has_ink_near(image: Image.Image, x: int, y: int, radius: int = 12) -> bool:
    for sample_y in range(max(0, y - radius), min(image.height, y + radius + 1)):
        for sample_x in range(max(0, x - radius), min(image.width, x + radius + 1)):
            if is_ink(image.getpixel((sample_x, sample_y))):
                return True
    return False


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("screenshot")
    for coordinate in ("x0", "y0", "x1", "y1"):
        parser.add_argument(coordinate, type=int)
    args = parser.parse_args()

    image = Image.open(args.screenshot).convert("RGB")
    endpoints = ((args.x0, args.y0), (args.x1, args.y1))
    missing = [point for point in endpoints if not has_ink_near(image, *point)]
    if missing:
        raise SystemExit(f"FAIL: no orange ink near input endpoints {missing}")
    print(f"PASS: orange ink covers input endpoints {endpoints}")


if __name__ == "__main__":
    main()
