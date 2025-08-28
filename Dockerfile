FROM python:3.13-slim

ENV PYTHONDONTWRITEBYTECODE=1
ENV PYTHONUNBUFFERED=1

COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/

RUN useradd -ms /bin/sh -u 1001 appuser
USER appuser

WORKDIR /app

COPY --chown=appuser:appuser pyproject.toml uv.lock* /app/
COPY --chown=appuser:appuser ./app/main.py /app/app.py

RUN uv sync --frozen --no-dev

EXPOSE 8383
CMD ["uv", "run", "uvicorn", "app:app", "--host", "0.0.0.0", "--port", "8383"]

