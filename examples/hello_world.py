import re
import ollama
import drun
import textwrap


def extract_code(text: str) -> str:
    text = re.sub(r'<think>.*?</think>', '', text, flags=re.DOTALL).strip()
    match = re.search(r'```(?:python)?\n?(.*?)```', text, re.DOTALL)
    return match.group(1).strip() if match else text.strip()


def main():
    system_prompt = textwrap.dedent("""
    You are a specialized Python coding agent. You have access to a secure WASM
    sandbox. Any code you write will be executed in a directory called
    '/workspace'.
                                    
    To modify files, write a Python script that reads and writes to '/workspace'.
    Your entire response is passed directly to Python's exec(). Any non-code
    characters will raise a SyntaxError and abort execution.
    """)
    prompt = 'Append a random number from 1-100 to a hello.txt file.'
    response = ollama.chat(model='deepseek-r1', messages=[
        {'role': 'system', 'content': system_prompt},
        {'role': 'user', 'content': prompt}
    ])
    code = extract_code(response['message']['content'])
    drun.execute(code, mounts=['examples/'])


if __name__ == '__main__':
    main()
