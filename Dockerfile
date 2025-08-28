FROM python:3.13-alpine AS builder

ENV PYTHONDONTWRITEBYTECODE=1
ENV PYTHONUNBUFFERED=1

COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/


WORKDIR /app

COPY --chown=1001:1001 pyproject.toml uv.lock* /app/
COPY --chown=1001:1001 ./app/main.py /app/app.py

RUN uv sync --frozen --no-dev --no-cache

FROM python:3.13-alpine

COPY --from=builder --chown=1001:1001 /app/.venv /app/.venv
COPY --from=builder --chown=1001:1001 /app/app.py /app/app.py
RUN mkdir /.cache && chown 1001:1001 /.cache
USER 1001:1001

WORKDIR /app
ENV VIRTUAL_ENV=/app/.venv
ENV PATH="$VIRTUAL_ENV/bin:$PATH"
EXPOSE 8383
CMD ["python", "-m", "uvicorn", "app:app", "--host", "0.0.0.0", "--port", "8383"]

