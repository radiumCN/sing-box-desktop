from PIL import Image
src = r'C:\Users\radium\.cursor\projects\c-RadiumCode-sing-box-win\assets\icon_source.png'
dst = r'C:\Users\radium\.cursor\projects\c-RadiumCode-sing-box-win\assets\icon_1024.png'
img = Image.open(src).convert("RGBA")
w, h = img.size
print("original size:", w, h)
side = min(w, h)
left = (w - side) // 2
top = (h - side) // 2
img_sq = img.crop((left, top, left + side, top + side)).resize((1024, 1024), Image.LANCZOS)
img_sq.save(dst)
print("saved 1024x1024 to", dst)
