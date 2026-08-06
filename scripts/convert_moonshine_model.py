#!/usr/bin/env python3
"""Convert a HuggingFace Moonshine streaming model to the 4-file ONNX format expected by moonshine.cpp.

Requires: pip install useful-moonshine[onnx] transformers

Usage:
  python3 scripts/convert_moonshine_model.py --model UsefulSensors/moonshine-streaming-tiny --out models/moonshine-tiny
"""

import argparse
import os
import sys

def main():
    parser = argparse.ArgumentParser(description="Convert Moonshine model to moonshine.cpp ONNX format")
    parser.add_argument("--model", default="UsefulSensors/moonshine-streaming-tiny",
                        help="HuggingFace model ID or path")
    parser.add_argument("--out", default="models/moonshine-tiny",
                        help="Output directory for ONNX files")
    args = parser.parse_args()

    os.makedirs(args.out, exist_ok=True)

    try:
        from moonshine.model_runner import MoonshineModel
    except ImportError:
        print("ERROR: useful-moonshine package not installed.", file=sys.stderr)
        print("Run: pip install 'useful-moonshine[onnx]'", file=sys.stderr)
        sys.exit(1)

    print(f"Loading model: {args.model}")
    model = MoonshineModel.from_pretrained(args.model)

    print(f"Exporting ONNX models to: {args.out}")
    model.export_onnx(args.out)

    # Verify
    expected = ["preprocess.onnx", "encode.onnx", "uncached_decode.onnx", "cached_decode.onnx"]
    missing = [f for f in expected if not os.path.exists(os.path.join(args.out, f))]
    if missing:
        print(f"WARNING: missing expected files: {missing}", file=sys.stderr)
        print("The moonshine.cpp library expects these exact filenames.", file=sys.stderr)
    else:
        print(f"Success! Model ready at {args.out}/")

    # Also copy tokenizer if present
    import shutil
    for fname in ["tokenizer.json", "tokenizer_config.json"]:
        src = os.path.join(args.model, fname) if os.path.isdir(args.model) else None
        if src and os.path.exists(src):
            shutil.copy2(src, os.path.join(args.out, fname))
            print(f"  Copied {fname}")

if __name__ == "__main__":
    main()
