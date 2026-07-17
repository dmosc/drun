"""
Data science agent — Iris classification benchmark.

The host fetches the Iris dataset from GitHub and pushes it into the sandbox
as a file. An LLM agent then implements and trains three classifiers from
scratch using only the Python standard library, compares their accuracy,
and exports a Markdown report with the results.

session_bash has no network access and no package-install mechanism, so
there is no pandas/scikit-learn available inside the sandbox — the agent
implements decision tree, random forest, and KNN classifiers itself using
plain Python.

Prerequisites:
    pip install 'drun-sandbox[chat]'
    # One of: ANTHROPIC_API_KEY, OPENAI_API_KEY, GEMINI_API_KEY

Usage:
    # Anthropic (recommended)
    DRUN_CONFIG=examples/data_science.toml \\
        python examples/data_science.py

    # OpenAI
    DRUN_CONFIG=examples/data_science.toml \\
        MODEL=gpt-4o python examples/data_science.py

    # Local Ollama (no API key)
    DRUN_CONFIG=examples/data_science.toml \\
        MODEL=openai/qwen2.5:14b BASE_URL=http://localhost:11434/v1 \\
        python examples/data_science.py

Expected behavior:
    1. Host fetches iris.csv from raw.githubusercontent.com and writes it
       into the session.
    2. Agent implements a decision tree, a small random forest, and a KNN
       classifier from scratch (stdlib only) and trains all three.
    3. Reports accuracy and a feature-importance ranking.
    4. Writes results.md, and the script exports it to
       /tmp/drun-data-science/results.md.
"""

import asyncio
import os
import textwrap
import urllib.request

from drun import Session
from drun.chat import ChatAgent, LocalSessionBridge

IRIS_URL = "https://raw.githubusercontent.com/mwaskom/seaborn-data/master/iris.csv"

PROMPT = textwrap.dedent("""\
    You are a data science agent operating inside a drun sandbox.
    iris.csv is already present in your workspace — it was fetched by the
    host before this session started (the sandbox itself has no network
    access, and there is no way to install pandas or scikit-learn inside
    it). Your task: implement three classifiers from scratch using only the
    Python standard library, compare their accuracy, and write a Markdown
    report.

    STEP 1 — Load the dataset
    --------------------------
    Parse iris.csv with the csv module. Use species as the label, the four
    measurement columns as features. Shuffle with random.seed(42) and split
    80/20 train/test.

    STEP 2 — Implement and train three classifiers (stdlib only)
    --------------------------------------------------------------
    a. A decision tree classifier: recursively split on the single
       feature/threshold pair that minimizes Gini impurity, to a max depth
       of 4.
    b. A small random forest: 10 decision trees (same algorithm as above),
       each trained on a bootstrap sample of the training set with a random
       subset of 2 features considered per split; predict by majority vote.
    c. A KNN classifier with k=5, using Euclidean distance over the four
       features.

    For each, compute accuracy on the test set. For the random forest, also
    compute a feature-importance ranking (count how often each feature is
    chosen as a split point across all trees, normalized).

    STEP 3 — Write the report
    ---------------------------
    Write results.md with:

    # Iris Classification Benchmark

    ## Classifier accuracy

    | Classifier      | Test accuracy |
    |-----------------|---------------|
    | Decision Tree   | X.XX          |
    | Random Forest   | X.XX          |
    | KNN (k=5)       | X.XX          |

    ## Random Forest — feature importances

    | Rank | Feature        | Importance |
    |------|----------------|------------|
    | 1    | petal_length   | 0.XX       |
    ...

    Add a one-sentence conclusion naming the best classifier.
""")


def main():
    model = os.environ.get("MODEL", "claude-sonnet-4-6")
    base_url = os.environ.get("BASE_URL")

    request = urllib.request.Request(
        IRIS_URL, headers={"User-Agent": "drun-example research@example.com"}
    )
    with urllib.request.urlopen(request) as response:
        iris_csv = response.read()

    session = Session()
    session.write_file("iris.csv", iris_csv)

    agent = ChatAgent(
        LocalSessionBridge(session),
        model=model,
        base_url=base_url,
        max_iterations=30,
    )
    asyncio.run(agent.run(PROMPT))

    export_dir = "/tmp/drun-data-science"
    exported = session.export(export_dir)
    if exported:
        print(f"\nExported: {exported}")
        print(f"Report: {export_dir}/results.md")


if __name__ == "__main__":
    main()
