import yaml

# Load the configuration
with open('config.yaml', 'r') as file:
    config = yaml.safe_load(file)

# Define the team class
class Contributor:
    def __init__(self, name, email):
        self.name = name
        self.email = email
        self.contributions = []

    def add_contribution(self, contribution):
        self.contributions.append(contribution)

class Contribution:
    def __init__(self, title, description, contributor):
        self.title = title
        self.description = description
        self.contributor = contributor
        self.status = "proposed"
        self.agreement_signed = False

    def sign_agreement(self):
        self.agreement_signed = True
        self.status = "accepted"

class Team:
    def __init__(self, config):
        self.config = config
        self.contributors = []
        self.contributions = []

    def add_contributor(self, contributor):
        self.contributors.append(contributor)

    def add_contribution(self, contribution):
        if contribution.contributor in self.contributors:
            self.contributions.append(contribution)
        else:
            raise ValueError("Contributor not part of the team")

    def review_contribution(self, contribution):
        if contribution.agreement_signed:
            contribution.status = "approved"
        else:
            raise ValueError("Contribution agreement must be signed before approval")

    def get_team_info(self):
        return {
            "team_name": self.config['platform']['team_name'],
            "official_domain": self.config['platform']['official_domain'],
            "organizer": self.config['external_competition']['organizer'],
            "presented_by": self.config['external_competition']['presented_by'],
            "official_url": self.config['external_competition']['official_url'],
            "official_rules_url": self.config['external_competition']['official_rules_url'],
            "advertised_total_prize_pool": self.config['external_competition']['advertised_total_prize_pool']
        }

# Example usage
if __name__ == "__main__":
    # Create the team
    team = Team(config)

    # Add contributors
    contributor1 = Contributor("Alice", "alice@example.com")
    contributor2 = Contributor("Bob", "bob@example.com")
    team.add_contributor(contributor1)
    team.add_contributor(contributor2)

    # Add contributions
    contribution1 = Contribution("Concept A", "Description of Concept A", contributor1)
    contribution2 = Contribution("Concept B", "Description of Concept B", contributor2)
    team.add_contribution(contribution1)
    team.add_contribution(contribution2)

    # Sign agreements
    contribution1.sign_agreement()
    contribution2.sign_agreement()

    # Review contributions
    team.review_contribution(contribution1)
    team.review_contribution(contribution2)

    # Get team info
    print(team.get_team_info())
