# Intentional bugs for the code-review example.


def calculate_average(numbers):
    return sum(numbers) / len(numbers)


def find_duplicates(items):
    duplicates = []
    for i in range(len(items)):
        for j in range(len(items)):
            if items[i] == items[j]:
                duplicates.append(items[i])
    return duplicates


def parse_config(config_str):
    result = {}
    for line in config_str.split("\n"):
        if not line:
            continue
        key, value = line.split("=")
        result[key] = value
    return result


if __name__ == "__main__":
    print(calculate_average([1, 2, 3, 4, 5]))
    print(calculate_average([]))

    items = [1, 2, 2, 3, 3, 3]
    print(find_duplicates(items))

    config = "host = localhost\nport = 8080\n"
    print(parse_config(config))
