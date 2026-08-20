from pathlib import Path
from PIL import Image, ImageDraw

out = Path('/tmp/panora-video-assets')
out.mkdir(parents=True, exist_ok=True)
image = Image.new('RGB', (640, 360), '#18222d')
draw = ImageDraw.Draw(image)
draw.rectangle((32, 32, 608, 328), outline='#4cc2ff', width=6)
draw.ellipse((220, 80, 420, 280), fill='#ffb347', outline='#ffffff', width=5)
draw.text((180, 300), 'Panora image format test', fill='#ffffff')
image.save(out / 'panora-test.png', format='PNG')
(out / 'sample.txt').write_text('Panora file-list test\n', encoding='utf-8')
(out / 'uris.txt').write_text(f'file://{out / "sample.txt"}\n', encoding='utf-8')
