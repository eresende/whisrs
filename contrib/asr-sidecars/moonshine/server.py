#!/usr/bin/env python3
"""HTTP sidecar for Useful Sensors Moonshine ASR.

The whisrs `asr-sidecar` backend posts WAV audio to `/transcribe` as multipart
form data. This sidecar loads a Moonshine model once at startup and returns a
plain transcript as JSON.
"""

from __future__ import annotations

import argparse
import os
import tempfile
from contextlib import asynccontextmanager
from pathlib import Path

import librosa
import torch
from fastapi import FastAPI, File, Form, HTTPException, UploadFile
from transformers import AutoProcessor, MoonshineForConditionalGeneration


DEFAULT_MODEL = "UsefulSensors/moonshine-base"


def _torch_dtype(name: str) -> torch.dtype:
    try:
        return getattr(torch, name)
    except AttributeError as exc:
        raise ValueError(f"unknown torch dtype: {name}") from exc


class MoonshineSidecar:
    def __init__(
        self,
        model_id: str,
        device: str,
        dtype: torch.dtype,
        max_length: int | None,
    ) -> None:
        self.model_id = model_id
        self.device = torch.device(device)
        self.dtype = dtype if self.device.type != "cpu" else torch.float32
        self.max_length = max_length

        self.processor = AutoProcessor.from_pretrained(model_id)
        self.model = MoonshineForConditionalGeneration.from_pretrained(model_id)
        self.model.to(self.device)
        self.model.to(self.dtype)
        self.model.eval()

    @torch.inference_mode()
    def transcribe(self, audio_path: Path) -> str:
        sampling_rate = self.processor.feature_extractor.sampling_rate
        audio, _ = librosa.load(audio_path, sr=sampling_rate, mono=True)
        inputs = self.processor(
            audio,
            return_tensors="pt",
            sampling_rate=sampling_rate,
        )
        inputs = inputs.to(self.device, self.dtype)

        generate_kwargs: dict[str, int] = {}
        if self.max_length is not None:
            generate_kwargs["max_length"] = self.max_length
        elif "attention_mask" in inputs:
            # Moonshine recommends capping generated length based on input
            # duration to avoid hallucination loops on short audio.
            token_limit_factor = 6.5 / sampling_rate
            seq_lens = inputs.attention_mask.sum(dim=-1)
            generate_kwargs["max_length"] = max(
                1,
                int((seq_lens * token_limit_factor).max().item()),
            )

        generated_ids = self.model.generate(**inputs, **generate_kwargs)
        return self.processor.decode(generated_ids[0], skip_special_tokens=True).strip()


def create_app(sidecar: MoonshineSidecar) -> FastAPI:
    @asynccontextmanager
    async def lifespan(app: FastAPI):
        app.state.sidecar = sidecar
        yield

    app = FastAPI(title="whisrs Moonshine ASR sidecar", lifespan=lifespan)

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
        if language and language not in {"auto", "en"}:
            raise HTTPException(
                status_code=400,
                detail="Moonshine English models only support language=en or auto",
            )
        # The generic sidecar contract includes hotwords/prompt, but Moonshine
        # does not currently consume them through the Transformers API.
        _ = hotwords or prompt

        suffix = Path(file.filename or "audio.wav").suffix or ".wav"
        with tempfile.NamedTemporaryFile(suffix=suffix, delete=False) as tmp:
            tmp_path = Path(tmp.name)
            tmp.write(await file.read())

        try:
            text = app.state.sidecar.transcribe(tmp_path)
        finally:
            tmp_path.unlink(missing_ok=True)

        return {"text": text}

    return app


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default=os.getenv("MOONSHINE_HOST", "127.0.0.1"))
    parser.add_argument("--port", type=int, default=int(os.getenv("MOONSHINE_PORT", "8765")))
    parser.add_argument("--model", default=os.getenv("MOONSHINE_MODEL", DEFAULT_MODEL))
    parser.add_argument(
        "--device",
        default=os.getenv(
            "MOONSHINE_DEVICE",
            "cuda:0" if torch.cuda.is_available() else "cpu",
        ),
    )
    parser.add_argument("--dtype", default=os.getenv("MOONSHINE_DTYPE", "float16"))
    parser.add_argument(
        "--max-length",
        type=int,
        default=(
            int(os.environ["MOONSHINE_MAX_LENGTH"])
            if "MOONSHINE_MAX_LENGTH" in os.environ
            else None
        ),
        help="Override generated token limit. By default this is derived from audio length.",
    )
    args = parser.parse_args()

    import uvicorn

    sidecar = MoonshineSidecar(
        model_id=args.model,
        device=args.device,
        dtype=_torch_dtype(args.dtype),
        max_length=args.max_length,
    )
    uvicorn.run(create_app(sidecar), host=args.host, port=args.port)


if __name__ == "__main__":
    main()
