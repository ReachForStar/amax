import formatTest from './Format.test';
import usageTest from './Usage.test';
import alertTest from './Alert.test';

export default function testsuite() {
  formatTest();
  usageTest();
  alertTest();
}
