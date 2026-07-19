import requests

def check_api_endpoints(base_url):
    endpoints = [
        "/health",
        "/v1/readiness/live-money",
        "/v1/bounties/funding-feed"
    ]
    
    for endpoint in endpoints:
        url = f"{base_url}{endpoint}"
        try:
            response = requests.get(url)
            if response.status_code == 200:
                print(f"Endpoint {url} is working: {response.text}")
            else:
                print(f"Endpoint {url} returned status code {response.status_code}: {response.text}")
        except requests.exceptions.RequestException as e:
            print(f"Error accessing {url}: {e}")

if __name__ == "__main__":
    base_url = "https://agent-bounties-api.onrender.com"
    check_api_endpoints(base_url)