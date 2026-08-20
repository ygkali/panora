from PIL import Image, ImageDraw

image = Image.new("RGB", (640, 360), (32, 78, 140))
draw = ImageDraw.Draw(image)
draw.rounded_rectangle((36, 36, 604, 324), radius=28, fill=(242, 247, 252), outline=(94, 145, 205), width=6)
draw.ellipse((92, 100, 242, 250), fill=(255, 192, 72))
draw.polygon([(285, 252), (380, 118), (470, 252)], fill=(44, 174, 117))
draw.text((80, 270), "Panora image preview", fill=(32, 78, 140))
image.save("/home/ubuntu/panora/test-artifacts/panora-runtime-photo.png", "PNG")
