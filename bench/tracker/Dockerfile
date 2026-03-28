FROM python:3.12-alpine
COPY tracker.py /app/tracker.py
EXPOSE 6969
HEALTHCHECK --interval=3s --timeout=2s --retries=5 \
    CMD python -c "import urllib.request; urllib.request.urlopen('http://localhost:6969/health')"
CMD ["python", "-u", "/app/tracker.py"]
