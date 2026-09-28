class Account:
    def __init__(self, owner, balance=0):
        self.owner = owner
        self.balance = balance
        self._history = [balance]

    def deposit(self, amount):
        self.balance += amount
        self._history.append(self.balance)
        return self.balance

    def withdraw(self, amount):
        if amount > self.balance:
            raise ValueError("insufficient funds")
        self.balance -= amount
        self._history.append(self.balance)
        return self.balance

    def history(self):
        return list(self._history)
