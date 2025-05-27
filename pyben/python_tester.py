import pyben

if __name__ == "__main__":
    for line in pyben.PyBenDecoder("../dev_files/tests/7x7_test2.jsonl.ben"):
        print(line)
