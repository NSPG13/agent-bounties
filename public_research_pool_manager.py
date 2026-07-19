import os
import json
from datetime import datetime

# Define the central repository directory
REPOSITORY_DIR = 'public_research_pool'
LOGS_DIR = 'logs'

# Create directories if they don't exist
os.makedirs(REPOSITORY_DIR, exist_ok=True)
os.makedirs(LOGS_DIR, exist_ok=True)

# Define the neutral template for outreach
OUTREACH_TEMPLATE = """
Dear {name},

We are reaching out to you regarding the XPRIZE Quantum Applications Phase II competition. We have a public, reproducible research pool that may be of interest to your team. 

Please review the following artifacts:
- Artifact 1: {artifact1}
- Artifact 2: {artifact2}

If you are interested in adopting any of these artifacts, please let us know. If not, you can opt-out by replying to this message.

Best regards,
[Your Name]
"""

# Function to log interactions
def log_interaction(team, action, artifact=None):
    timestamp = datetime.now().strftime('%Y-%m-%d %H:%M:%S')
    log_entry = {
        'timestamp': timestamp,
        'team': team,
        'action': action,
        'artifact': artifact
    }
    log_file = os.path.join(LOGS_DIR, f'{team}_log.json')
    with open(log_file, 'a') as f:
        json.dump(log_entry, f)
        f.write('\n')

# Function to add a new artifact to the repository
def add_artifact(artifact_name, artifact_data):
    artifact_path = os.path.join(REPOSITORY_DIR, artifact_name)
    with open(artifact_path, 'w') as f:
        f.write(artifact_data)
    log_interaction('System', 'Artifact Added', artifact_name)

# Function to list all artifacts in the repository
def list_artifacts():
    return os.listdir(REPOSITORY_DIR)

# Function to send an outreach message to a team
def send_outreach_message(team, artifacts):
    message = OUTREACH_TEMPLATE.format(
        name=team['name'],
        artifact1=artifacts[0],
        artifact2=artifacts[1]
    )
    print(f"Sending outreach message to {team['name']}:\n{message}")
    log_interaction(team['name'], 'Outreach Sent', artifacts)

# Example teams (finalists)
teams = [
    {'name': 'Team A', 'contact': 'teamA@example.com'},
    {'name': 'Team B', 'contact': 'teamB@example.com'},
    {'name': 'Team C', 'contact': 'teamC@example.com'}
]

# Example usage
if __name__ == '__main__':
    # Add some artifacts to the repository
    add_artifact('artifact1.txt', 'This is the content of artifact 1.')
    add_artifact('artifact2.txt', 'This is the content of artifact 2.')

    # List all artifacts
    artifacts = list_artifacts()
    print("Available artifacts:", artifacts)

    # Send outreach messages to all teams
    for team in teams:
        send_outreach_message(team, artifacts)

    # Log an example interaction
    log_interaction('Team A', 'Artifact Adopted', 'artifact1.txt')
