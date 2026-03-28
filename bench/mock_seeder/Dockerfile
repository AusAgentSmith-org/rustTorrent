FROM python:3.12-alpine
COPY mock_seeder.py /app/mock_seeder.py
WORKDIR /app
HEALTHCHECK --interval=5s --timeout=3s --retries=10 \
    CMD python -c "import urllib.request; urllib.request.urlopen('http://localhost:8080/')"
CMD ["python", "-u", "mock_seeder.py"]
