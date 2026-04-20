import ollama
import drun
import textwrap


def main():
    system_prompt = textwrap.dedent("""
    You are a specialized Python coding agent. You have access to a secure WASM
    sandbox. Any code you write will be executed in a directory called
    '/workspace'.
                                    
    To modify files, write a Python script that reads/ writes to '/workspace'.
    Return only the Python code block, nothing else. Not even markdown block. I
    want the raw Python code to execute.
    """)
    prompt = 'Append a random number from 1-100 to a hello.txt file.'
    response = ollama.chat(model='deepseek-r1', messages=[
        {'role': 'system', 'content': system_prompt},
        {'role': 'user', 'content': prompt}
    ])
    code = response['message']['content']
    drun.execute(code, mounts=['examples/'])


if __name__ == '__main__':
    main()
