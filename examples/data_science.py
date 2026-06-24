"""
Data science agent — Iris classification benchmark.

An LLM agent fetches the Iris dataset from GitHub, installs scikit-learn and
tabulate inside the sandbox, trains three classifiers, compares their accuracy,
and exports a Markdown report with the results.

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
    1. Agent fetches iris.csv from raw.githubusercontent.com.
    2. Installs pandas, scikit-learn, and tabulate inside the sandbox.
    3. Trains Decision Tree, Random Forest, and KNN classifiers.
    4. Reports accuracy and a feature-importance table.
    5. Writes results.md to /workspace; the script exports it to
       /tmp/drun-data-science/results.md.
"""

import os
import textwrap

from drun import Session
from drun.chat import run

PROMPT = textwrap.dedent("""\
    You are a data science agent operating inside a drun sandbox.
    Your task: fetch the Iris dataset, train three classifiers, compare their
    accuracy, and write a Markdown report.

    STEP 1 — Fetch the dataset
    --------------------------
    Use urllib.request (no install needed) to download:

        https://raw.githubusercontent.com/mwaskom/seaborn-data/master/iris.csv

    Save it to /workspace/iris.csv.

    STEP 2 — Install packages
    -------------------------
    install_package("pandas")
    install_package("scikit-learn")
    install_package("tabulate")

    STEP 3 — Train and evaluate classifiers
    ----------------------------------------
    Load iris.csv with pandas. Use species as the label, the four measurement
    columns as features. Split 80/20 train/test with random_state=42.

    Train these three classifiers (all from sklearn):
      a. DecisionTreeClassifier(random_state=42)
      b. RandomForestClassifier(n_estimators=100, random_state=42)
      c. KNeighborsClassifier(n_neighbors=5)

    For each, compute accuracy on the test set. For the Random Forest, also
    extract feature importances and rank the four features.

    STEP 4 — Write the report
    -------------------------
    Write /workspace/results.md with:

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

    Do not snapshot — the config handles persistence settings.
""")


def main():
    model = os.environ.get("MODEL", "claude-sonnet-4-6")
    base_url = os.environ.get("BASE_URL")

    session = Session()

    run(
        session,
        PROMPT,
        model=model,
        base_url=base_url,
        max_iterations=30,
    )

    export_dir = "/tmp/drun-data-science"
    exported = session.export(export_dir)
    if exported:
        print(f"\nExported: {exported}")
        print(f"Report: {export_dir}/results.md")


if __name__ == "__main__":
    main()
