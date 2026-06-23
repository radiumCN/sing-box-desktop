from PIL import Image

src = r'C:\Users\radium\.cursor\projects\c-RadiumCode-sing-box-win\assets\icon_refined.png'
dst = r'C:\Users\radium\.cursor\projects\c-RadiumCode-sing-box-win\assets\icon_final_1024.png'

img = Image.open(src).convert("RGBA")
w, h = img.size
print(f"original size: {w}x{h}")

# Crop to center square
side = min(w, h)
left = (w - side) // 2
top  = (h - side) // 2
img_sq = img.crop((left, top, left + side, top + side))

# Upscale/downscale to exactly 1024x1024
img_1024 = img_sq.resize((1024, 1024), Image.LANCZOS)
img_1024.save(dst)
print(f"saved {dst}")
