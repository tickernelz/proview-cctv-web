#!/usr/bin/env python3
import colorsys
import math
import sys

WIDTH = 512
HEIGHT = 512
CENTER_X = 256.0
CENTER_Y = 256.0
RADIUS = 240.0
MAX_THETA_DEG = 90.0
SUPERSAMPLE = 4

SPOKE_BEARINGS = [15.0 + index * 30.0 for index in range(12)]
CIRCLE_THETAS = [15.0, 30.0, 45.0, 60.0, 75.0, 90.0]
CIRCLE_GRAYS = [96, 112, 128, 144, 160, 255]
BACKGROUND = (8, 10, 16)
SPOKE_INNER = 0.10
SPOKE_OUTER = 0.96
SPOKE_HALF_WIDTH = 2.2
CIRCLE_HALF_WIDTH = 1.6


def spoke_color(index):
    hue = index / len(SPOKE_BEARINGS)
    red, green, blue = colorsys.hsv_to_rgb(hue, 1.0, 1.0)
    return (int(red * 255), int(green * 255), int(blue * 255))


def sample(x, y):
    dx = x - CENTER_X
    dy = y - CENTER_Y
    radius = math.hypot(dx, dy)
    bearing = math.degrees(math.atan2(dy, dx)) % 360.0
    theta = radius / RADIUS * MAX_THETA_DEG
    best = BACKGROUND
    best_distance = None
    for half_width, theta_center, gray in [
        (CIRCLE_HALF_WIDTH, value, color)
        for value, color in zip(CIRCLE_THETAS, CIRCLE_GRAYS)
    ]:
        distance = abs(theta - theta_center) * RADIUS / MAX_THETA_DEG
        if distance <= half_width and radius <= RADIUS + CIRCLE_HALF_WIDTH:
            if best_distance is None or distance < best_distance:
                best = (gray, gray, gray)
                best_distance = distance
    for index, center in enumerate(SPOKE_BEARINGS):
        delta = abs((bearing - center + 180.0) % 360.0 - 180.0)
        distance = delta * math.pi / 180.0 * radius
        inside = SPOKE_INNER * RADIUS <= radius <= SPOKE_OUTER * RADIUS
        if inside and distance <= SPOKE_HALF_WIDTH:
            if best_distance is None or distance < best_distance:
                best = spoke_color(index)
                best_distance = distance
    return best


def main(path):
    step = 1.0 / SUPERSAMPLE
    payload = bytearray()
    for row in range(HEIGHT):
        for column in range(WIDTH):
            red = green = blue = 0
            for sub_row in range(SUPERSAMPLE):
                for sub_column in range(SUPERSAMPLE):
                    color = sample(
                        column + (sub_column + 0.5) * step,
                        row + (sub_row + 0.5) * step,
                    )
                    red += color[0]
                    green += color[1]
                    blue += color[2]
            divisor = SUPERSAMPLE * SUPERSAMPLE
            payload += bytes((red // divisor, green // divisor, blue // divisor))
    with open(path, "wb") as handle:
        handle.write(b"P6\n%d %d\n255\n" % (WIDTH, HEIGHT))
        handle.write(payload)
    print(path, WIDTH, HEIGHT, len(payload))


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "fisheye-grid-512.ppm")
