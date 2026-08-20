#!/usr/bin/env python3
import os
from PIL import ImageGrab

output = os.environ["PANORA_SCREENSHOT"]
ImageGrab.grab().save(output)
print(output)
