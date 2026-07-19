import requests

def fetch_rss_feed():
    response = requests.get('http://localhost:5000/llms.rss')
    with open('sample.rss', 'w') as f:
        f.write(response.text)

def fetch_json_feed():
    response = requests.get('http://localhost:5000/llms.json')
    with open('sample.json', 'w') as f:
        f.write(response.text)

if __name__ == '__main__':
    fetch_rss_feed()
    fetch_json_feed()