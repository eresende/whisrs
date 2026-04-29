#!/usr/bin/env python3
"""HTTP sidecar for Microsoft VibeVoice-ASR.

The whisrs `asr-sidecar` backend posts WAV audio to `/transcribe` as multipart
form data. This sidecar loads the Transformers-compatible VibeVoice-ASR model
once at startup and returns a flattened transcript as JSON.
"""

from __future__ import annotations

import argparse
import os
import tempfile
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any

import torch
from fastapi import FastAPI, File, Form, HTTPException, UploadFile
from transformers import AutoProcessor, VibeVoiceAsrForConditionalGeneration


DEFAULT_MODEL = "microsoft/VibeVoice-ASR-HF"


def _torch_dtype(name: str) -> torch.dtype | str:
    if name == "auto":
        return "auto"
    try:
        return getattr(torch, name)
    except AttributeError as exc:
        raise ValueError(f"unknown torch dtype: {name}") from exc


def _model_device(model: Any) -> torch.device:
    if hasattr(model, "device"):
        return model.device
    first_param = next(model.parameters())
    return first_param.device


class VibeVoiceSidecar:
    def __init__(
        self,
        model_id: str,
        device_map: str,
        dtype: torch.dtype | str,
        tokenizer_chunk_size: int | None,
        max_new_tokens: int | None,
    ) -> None:
        self.model_id = model_id
        self.tokenizer_chunk_size = tokenizer_chunk_size
        self.max_new_tokens = max_new_tokens
        self.processor = AutoProcessor.from_pretrained(model_id)
        self.model = VibeVoiceAsrForConditionalGeneration.from_pretrained(
            model_id,
            device_map=device_map,
            torch_dtype=dtype,
        )

    @torch.inference_mode()
    def transcribe(self, audio_path: Path, prompt: str | None) -> str:
        inputs = self.processor.apply_transcription_request(
            audio=str(audio_path),
            prompt=prompt or None,
        )

        device = _model_device(self.model)
        model_dtype = getattr(self.model, "dtype", torch.float32)
        inputs = inputs.to(device, model_dtype)

        generate_kwargs: dict[str, Any] = {}
        if self.tokenizer_chunk_size is not None:
            generate_kwargs["tokenizer_chunk_size"] = self.tokenizer_chunk_size
        if self.max_new_tokens is not None:
            generate_kwargs["max_new_tokens"] = self.max_new_tokens

        try:
            output_ids = self.model.generate(**inputs, **generate_kwargs)
        except ValueError as exc:
            if (
                "tokenizer_chunk_size" not in generate_kwargs
                or "tokenizer_chunk_size" not in str(exc)
            ):
                raise
            generate_kwargs.pop("tokenizer_chunk_size")
            output_ids = self.model.generate(**inputs, **generate_kwargs)
        generated_ids = output_ids[:, inputs["input_ids"].shape[1] :]
        transcript = self.processor.decode(
            generated_ids,
            return_format="transcription_only",
        )[0]
        return str(transcript).strip()


def create_app(sidecar: VibeVoiceSidecar) -> FastAPI:
    @asynccontextmanager
    async def lifespan(app: FastAPI):
        app.state.sidecar = sidecar
        yield

    app = FastAPI(title="whisrs VibeVoice-ASR sidecar", lifespan=lifespan)

    @app.get("/health")
    async def health() -> dict[str, str]:
        return {"status": "ok", "model": app.state.sidecar.model_id}

    @app.post("/transcribe")
    async def transcribe(
        file: UploadFile = File(...),
        model: str = Form(DEFAULT_MODEL),
        language: str | None = Form(None),
        hotwords: str | None = Form(None),
        prompt: str | None = Form(None),
    ) -> dict[str, str]:
        if model != app.state.sidecar.model_id:
            raise HTTPException(
                status_code=400,
                detail=(
                    f"sidecar loaded {app.state.sidecar.model_id}, "
                    f"but request asked for {model}"
                ),
            )
        if language and language != "auto":
            # VibeVoice-ASR is multilingual and does not require an explicit
            # language parameter. Keep accepting the form field so whisrs can
            # use the same transcription config shape as other backends.
            pass

        suffix = Path(file.filename or "audio.wav").suffix or ".wav"
        with tempfile.NamedTemporaryFile(suffix=suffix, delete=False) as tmp:
            tmp_path = Path(tmp.name)
            tmp.write(await file.read())

        try:
            text = app.state.sidecar.transcribe(tmp_path, prompt or hotwords)
        finally:
            tmp_path.unlink(missing_ok=True)

        return {"text": text}

    return app


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default=os.getenv("VIBEVOICE_HOST", "127.0.0.1"))
    parser.add_argument("--port", type=int, default=int(os.getenv("VIBEVOICE_PORT", "8765")))
    parser.add_argument("--model", default=os.getenv("VIBEVOICE_MODEL", DEFAULT_MODEL))
    parser.add_argument("--device-map", default=os.getenv("VIBEVOICE_DEVICE_MAP", "auto"))
    parser.add_argument("--dtype", default=os.getenv("VIBEVOICE_DTYPE", "float16"))
    parser.add_argument(
        "--tokenizer-chunk-size",
        type=int,
        default=(
            int(os.environ["VIBEVOICE_TOKENIZER_CHUNK_SIZE"])
            if "VIBEVOICE_TOKENIZER_CHUNK_SIZE" in os.environ
            else None
        ),
        help="Optional generate() tokenizer chunk size for lowering memory use.",
    )
    parser.add_argument(
        "--max-new-tokens",
        type=int,
        default=(
            int(os.environ["VIBEVOICE_MAX_NEW_TOKENS"])
            if "VIBEVOICE_MAX_NEW_TOKENS" in os.environ
            else 4096
        ),
        help="Maximum generated transcript tokens per request. Set 0 to use the model default.",
    )
    args = parser.parse_args()

    import uvicorn

    sidecar = VibeVoiceSidecar(
        model_id=args.model,
        device_map=args.device_map,
        dtype=_torch_dtype(args.dtype),
        tokenizer_chunk_size=args.tokenizer_chunk_size,
        max_new_tokens=args.max_new_tokens if args.max_new_tokens > 0 else None,
    )
    uvicorn.run(create_app(sidecar), host=args.host, port=args.port)


if __name__ == "__main__":
    main()
