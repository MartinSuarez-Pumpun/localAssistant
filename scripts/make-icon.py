#!/usr/bin/env python3
"""Generates a minimal 256x256 PNG icon for llm-assistant if none exists."""
import struct, zlib, sys, os

def create_png(width, height, pixels_rgb, out_path):
    def chunk(tag, data):
        c = struct.pack('>I', len(data)) + tag + data
        return c + struct.pack('>I', zlib.crc32(tag + data) & 0xffffffff)

    raw = b''
    for row in range(height):
        raw += b'\x00'  # filter type None
        for col in range(width):
            raw += bytes(pixels_rgb[row * width + col])

    ihdr = struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0)
    idat = zlib.compress(raw, 9)

    png = b'\x89PNG\r\n\x1a\n'
    png += chunk(b'IHDR', ihdr)
    png += chunk(b'IDAT', idat)
    png += chunk(b'IEND', b'')

    with open(out_path, 'wb') as f:
        f.write(png)

def main():
    out = sys.argv[1] if len(sys.argv) > 1 else 'scripts/llm-assistant.png'
    if os.path.exists(out):
        print(f"Icon already exists: {out}")
        return

    W, H = 256, 256
    pixels = []
    cx, cy = W // 2, H // 2
    r_outer = 110
    r_inner = 80

    for y in range(H):
        for x in range(W):
            dx, dy = x - cx, y - cy
            dist = (dx*dx + dy*dy) ** 0.5
            if dist <= r_outer:
                if dist <= r_inner:
                    # Interior: dark blue-grey
                    pixels.append((25, 28, 50))
                else:
                    # Ring: vivid blue
                    t = (dist - r_inner) / (r_outer - r_inner)
                    rv = int(50 + (1-t)*80)
                    gv = int(100 + (1-t)*120)
                    bv = int(220)
                    pixels.append((rv, gv, bv))
            else:
                # Transparent background → white for PNG RGB
                pixels.append((240, 240, 240))

    # Draw a simple "AI" text-like symbol (three horizontal bars)
    for bar_y in range(3):
        y0 = cy - 30 + bar_y * 25
        for y in range(y0, y0 + 14):
            if 0 <= y < H:
                for x in range(cx - 30, cx + 30):
                    if 0 <= x < W:
                        pixels[y * W + x] = (220, 240, 255)

    create_png(W, H, pixels, out)
    print(f"Created icon: {out}")

if __name__ == '__main__':
    main()
